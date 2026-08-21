//! The single model every view reads from.
//!
//! [`Snapshot`] owns the latest values, the rolling histories behind the
//! charts, and the small amount of state needed to derive rates (bitrate) and
//! session-relative counters (frames dropped *this stream*, not since OBS
//! launched). It performs no I/O and holds no handles, so the whole thing is
//! testable by feeding it synthetic polls.

use std::time::{Duration, Instant};

use crate::metrics::alerts::{Alert, AlertConfig, Counters, Watcher};
use crate::metrics::history::History;
use crate::obs::client::{FaderState, Inventory, LogLevel, MeterFrame, PollData, Update};
use crate::obs::stats::{
    AudioInput, FrameTally, RecordStatus, SceneList, Stats, StreamStatus, Version, VideoSettings,
};

/// How many log lines the event view retains.
pub const LOG_CAPACITY: usize = 500;

/// Where the connection currently stands, as the header renders it.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connecting {
        attempt: u32,
    },
    Connected,
    Disconnected {
        reason: String,
        hint: Option<&'static str>,
        retry_in: Duration,
    },
    /// Terminal failure — bad credentials, bad config. No retry is coming.
    Fatal {
        reason: String,
        hint: Option<&'static str>,
    },
}

impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Connected)
    }

    /// Short label for the header chip.
    pub fn label(&self) -> String {
        match self {
            ConnectionState::Connecting { attempt } if *attempt <= 1 => "CONNECTING".to_string(),
            ConnectionState::Connecting { attempt } => format!("RETRY #{attempt}"),
            ConnectionState::Connected => "CONNECTED".to_string(),
            ConnectionState::Disconnected { .. } => "OFFLINE".to_string(),
            ConnectionState::Fatal { .. } => "FAILED".to_string(),
        }
    }
}

/// One rendered log line.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    /// Seconds since the app started, for a monotonic timestamp column.
    pub at: Duration,
}

/// Overall health, shown as a single word so a glance is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Strained,
    Dropping,
}

impl Health {
    pub fn label(self) -> &'static str {
        match self {
            Health::Healthy => "HEALTHY",
            Health::Strained => "STRAINED",
            Health::Dropping => "DROPPING",
        }
    }
}

/// Rolling series behind the sparklines and charts.
#[derive(Debug, Clone)]
pub struct Histories {
    pub cpu: History,
    pub memory: History,
    pub fps: History,
    pub frame_time: History,
    pub stream_bitrate: History,
    pub record_bitrate: History,
    pub congestion: History,
    pub render_skips: History,
    pub output_skips: History,
}

impl Histories {
    pub fn new(capacity: usize) -> Self {
        Self {
            cpu: History::new(capacity),
            memory: History::new(capacity),
            fps: History::new(capacity),
            frame_time: History::new(capacity),
            stream_bitrate: History::new(capacity),
            record_bitrate: History::new(capacity),
            congestion: History::new(capacity),
            render_skips: History::new(capacity),
            output_skips: History::new(capacity),
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        for history in self.each_mut() {
            history.set_capacity(capacity);
        }
    }

    fn each_mut(&mut self) -> [&mut History; 9] {
        [
            &mut self.cpu,
            &mut self.memory,
            &mut self.fps,
            &mut self.frame_time,
            &mut self.stream_bitrate,
            &mut self.record_bitrate,
            &mut self.congestion,
            &mut self.render_skips,
            &mut self.output_skips,
        ]
    }
}

/// Counters captured when an output started, so the UI can report "dropped
/// during this stream" rather than "dropped since OBS launched".
#[derive(Debug, Clone, Copy, PartialEq)]
struct Baseline {
    render: FrameTally,
    output: FrameTally,
}

/// Byte counter and timestamp of the previous poll, for rate derivation.
#[derive(Debug, Clone, Copy)]
struct ByteMark {
    bytes: u64,
    at: Instant,
}

/// Everything the UI draws.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub connection: ConnectionState,
    pub version: Version,
    pub stats: Stats,
    pub stream: StreamStatus,
    pub record: RecordStatus,
    pub video: VideoSettings,
    pub scenes: SceneList,
    pub inputs: Vec<AudioInput>,
    pub profiles: Vec<String>,
    pub current_profile: String,
    pub collections: Vec<String>,
    pub current_collection: String,
    pub history: Histories,
    /// Derived stream bitrate, bits per second.
    pub stream_bitrate: f64,
    /// Derived recording write rate, bits per second.
    pub record_bitrate: f64,
    pub log: Vec<LogEntry>,
    /// Number of hot polls applied — the header's "live data" heartbeat.
    pub polls: u64,
    /// When the most recent poll landed.
    pub last_poll: Option<Instant>,

    /// Raises alerts when frames are actively being lost.
    alerts: Watcher,

    started: Instant,
    stream_baseline: Option<Baseline>,
    stream_mark: Option<ByteMark>,
    record_mark: Option<ByteMark>,
}

impl Snapshot {
    /// Create a snapshot retaining `history_capacity` samples per series.
    pub fn new(history_capacity: usize) -> Self {
        Self {
            connection: ConnectionState::Connecting { attempt: 1 },
            version: Version::default(),
            stats: Stats::default(),
            stream: StreamStatus::default(),
            record: RecordStatus::default(),
            video: VideoSettings::default(),
            scenes: SceneList::default(),
            inputs: Vec::new(),
            profiles: Vec::new(),
            current_profile: String::new(),
            collections: Vec::new(),
            current_collection: String::new(),
            history: Histories::new(history_capacity),
            stream_bitrate: 0.0,
            record_bitrate: 0.0,
            log: Vec::new(),
            polls: 0,
            last_poll: None,
            alerts: Watcher::new(AlertConfig::default()),
            started: Instant::now(),
            stream_baseline: None,
            stream_mark: None,
            record_mark: None,
        }
    }

    /// Replace the alert thresholds.
    pub fn set_alert_config(&mut self, config: AlertConfig) {
        self.alerts.set_config(config);
    }

    /// Fold one [`Update`] from the session task into the model.
    ///
    /// Returns any alerts the update raised, so the shell can forward them to
    /// the desktop notifier. They are already recorded in [`Snapshot::log`].
    pub fn apply(&mut self, update: Update, now: Instant) -> Vec<Alert> {
        match update {
            Update::Connecting { attempt } => {
                self.connection = ConnectionState::Connecting { attempt };
            }
            Update::Connected(version) => {
                let was_offline = !self.connection.is_connected();
                self.connection = ConnectionState::Connected;
                // GetVersion arrives after the handshake and replaces the
                // partial banner; don't let it blank what we already showed.
                if !version.obs_version.is_empty() {
                    self.version.obs_version = version.obs_version.clone();
                    self.version.platform = version.platform.clone();
                }
                if !version.websocket_version.is_empty() {
                    self.version.websocket_version = version.websocket_version.clone();
                }
                if was_offline {
                    self.push_log(LogLevel::Info, "connected to OBS".to_string(), now);
                }
            }
            Update::Poll(poll) => return self.apply_poll(*poll, now),
            Update::Inventory(inventory) => self.apply_inventory(*inventory),
            Update::Meters(frames) => self.apply_meters(&frames),
            Update::Faders(faders) => self.apply_faders(&faders),
            Update::Event { level, message } => self.push_log(level, message, now),
            Update::Disconnected {
                reason,
                hint,
                retry_in,
            } => {
                if self.connection.is_connected() {
                    self.push_log(LogLevel::Error, format!("disconnected: {reason}"), now);
                }
                self.connection = ConnectionState::Disconnected {
                    reason,
                    hint,
                    retry_in,
                };
                self.on_connection_lost();
            }
            Update::Fatal { reason, hint } => {
                self.push_log(LogLevel::Error, reason.clone(), now);
                self.connection = ConnectionState::Fatal { reason, hint };
                self.on_connection_lost();
            }
        }

        Vec::new()
    }

    fn apply_poll(&mut self, poll: PollData, now: Instant) -> Vec<Alert> {
        let stream_was_active = self.stream.active;
        let record_was_active = self.record.active;

        self.stats = poll.stats;
        self.stream = poll.stream;
        self.record = poll.record;
        self.polls += 1;
        self.last_poll = Some(now);

        // A stream that just started defines a new baseline for the
        // since-launch counters in GetStats.
        if self.stream.active && !stream_was_active {
            self.stream_baseline = Some(Baseline {
                render: self.stats.render(),
                output: self.stats.output(),
            });
            self.stream_mark = None;
        } else if !self.stream.active && stream_was_active {
            self.stream_baseline = None;
            self.stream_mark = None;
            self.stream_bitrate = 0.0;
        }

        if !self.record.active && record_was_active {
            self.record_mark = None;
            self.record_bitrate = 0.0;
        }

        self.stream_bitrate = if self.stream.active {
            derive_bitrate(&mut self.stream_mark, self.stream.bytes, now)
        } else {
            0.0
        };
        self.record_bitrate = if self.record.active && !self.record.paused {
            derive_bitrate(&mut self.record_mark, self.record.bytes, now)
        } else {
            0.0
        };

        self.history.cpu.push(self.stats.cpu_usage);
        self.history.memory.push(self.stats.memory_usage);
        self.history.fps.push(self.stats.active_fps);
        self.history
            .frame_time
            .push(self.stats.average_frame_render_time);
        self.history.stream_bitrate.push(self.stream_bitrate);
        self.history.record_bitrate.push(self.record_bitrate);
        self.history.congestion.push(self.stream.congestion * 100.0);
        self.history
            .render_skips
            .push(self.session_render_skipped() as f64);
        self.history
            .output_skips
            .push(self.session_output_skipped() as f64);

        // Fed after the counters land so the watcher sees the same numbers the
        // panels do, including the per-stream baselines.
        let alerts = self.alerts.observe(
            Counters {
                render_skipped: self.stats.render_skipped_frames,
                output_skipped: self.stats.output_skipped_frames,
                network_dropped: self.stream.skipped_frames,
                outputting: self.stream.active || self.record.active,
                reconnecting: self.stream.reconnecting,
            },
            now,
        );

        // Every alert also becomes a log line, so the Log view is a complete
        // record even for alerts whose banner has already expired.
        for alert in &alerts {
            let level = match alert.severity() {
                crate::metrics::alerts::Severity::Critical => LogLevel::Error,
                crate::metrics::alerts::Severity::Warning => LogLevel::Warn,
            };
            self.push_log(
                level,
                format!(
                    "{}: {}",
                    alert.kind.label().to_ascii_lowercase(),
                    alert.summary()
                ),
                now,
            );
        }

        alerts
    }

    fn apply_inventory(&mut self, inventory: Inventory) {
        self.video = inventory.video;
        self.scenes = inventory.scenes;
        self.profiles = inventory.profiles;
        self.current_profile = inventory.current_profile;
        self.collections = inventory.collections;
        self.current_collection = inventory.current_collection;

        // Merge rather than replace: the input list carries no levels, and
        // clobbering them would make the meters flicker to silence every
        // inventory tick.
        let mut merged = inventory.inputs;
        for input in &mut merged {
            if let Some(previous) = self.inputs.iter().find(|prev| prev.name == input.name) {
                input.peaks = previous.peaks.clone();
                input.muted = previous.muted;
                input.volume_db = previous.volume_db;
                input.volume_mul = previous.volume_mul;
            }
        }
        self.inputs = merged;
    }

    /// Apply either a meter event (peaks) or a mute/volume probe.
    ///
    /// Both arrive in the same `(name, values)` shape. A probe is identified
    /// by its two-slot payload whose entries are mute-as-`0.0`/`1.0` and a dB
    /// value; peaks are always linear `0.0..=1.0`. The dB slot is negative or
    /// NaN in practice, which is what disambiguates the two.
    /// Fresh peak levels. An input we have never heard of is skipped: the
    /// meter event can name an input that the next inventory poll has not
    /// introduced yet.
    fn apply_meters(&mut self, frames: &[MeterFrame]) {
        for frame in frames {
            let Some(input) = self.input_mut(&frame.name) else {
                continue;
            };
            input.peaks = frame.peaks.clone();
        }
    }

    /// Fresh mixer positions. `None` means OBS did not answer that half of
    /// the mute/volume pair, so the previous value stands rather than being
    /// clobbered with a zero.
    fn apply_faders(&mut self, faders: &[FaderState]) {
        for fader in faders {
            let Some(input) = self.input_mut(&fader.name) else {
                continue;
            };
            if let Some(muted) = fader.muted {
                input.muted = muted;
            }
            if let Some(db) = fader.volume_db {
                input.volume_db = db;
                input.volume_mul = db_to_mul(db);
            }
        }
    }

    fn input_mut(&mut self, name: &str) -> Option<&mut AudioInput> {
        self.inputs.iter_mut().find(|input| input.name == name)
    }

    fn on_connection_lost(&mut self) {
        self.stream_mark = None;
        self.record_mark = None;
        self.stream_bitrate = 0.0;
        self.record_bitrate = 0.0;
        // Silence the meters; leaving the last peaks on screen would imply
        // audio is still flowing.
        for input in &mut self.inputs {
            input.peaks.clear();
        }
    }

    fn push_log(&mut self, level: LogLevel, message: String, now: Instant) {
        if self.log.len() == LOG_CAPACITY {
            self.log.remove(0);
        }
        self.log.push(LogEntry {
            level,
            message,
            at: now.saturating_duration_since(self.started),
        });
    }

    /// Resize every history window.
    pub fn set_history_capacity(&mut self, capacity: usize) {
        self.history.set_capacity(capacity);
    }

    /// How long since the last poll landed — drives the "stale" indicator.
    pub fn age(&self, now: Instant) -> Option<Duration> {
        self.last_poll
            .map(|last| now.saturating_duration_since(last))
    }

    /// One counter pair measured over the current stream rather than since
    /// OBS launched.
    ///
    /// `pick` selects which pair to read, from both the live stats and the
    /// baseline captured when the stream started. With no stream running
    /// there is no baseline, and the since-launch total is the only
    /// meaningful number, so it is returned unchanged.
    fn session(
        &self,
        pick: fn(&Stats) -> FrameTally,
        of_baseline: fn(&Baseline) -> FrameTally,
    ) -> FrameTally {
        let live = pick(&self.stats);
        match &self.stream_baseline {
            Some(baseline) => live.since(of_baseline(baseline)),
            None => live,
        }
    }

    /// Frames the renderer skipped since the current stream started.
    ///
    /// Falls back to the since-launch total when no stream is running, which
    /// is the only meaningful number in that state.
    pub fn session_render_skipped(&self) -> u64 {
        self.session(Stats::render, |baseline| baseline.render)
            .skipped
    }

    /// Frames the encoder skipped since the current stream started.
    pub fn session_output_skipped(&self) -> u64 {
        self.session(Stats::output, |baseline| baseline.output)
            .skipped
    }

    /// Render-skip rate over the current stream, as a percentage.
    pub fn session_render_skip_percent(&self) -> f64 {
        self.session(Stats::render, |baseline| baseline.render)
            .percent()
    }

    /// Encoder-skip rate over the current stream, as a percentage.
    pub fn session_output_skip_percent(&self) -> f64 {
        self.session(Stats::output, |baseline| baseline.output)
            .percent()
    }

    /// Achieved FPS as a fraction of the configured target, `0.0..=1.0`.
    ///
    /// Returns 1.0 when the target is unknown, so the gauge reads "fine"
    /// rather than "catastrophic" before the first inventory poll lands.
    pub fn fps_ratio(&self) -> f64 {
        let target = self.video.target_fps();
        if target <= 0.0 {
            return 1.0;
        }
        (self.stats.active_fps / target).clamp(0.0, 1.0)
    }

    /// Average render time as a share of the frame budget, `0.0..=1.0`.
    ///
    /// Above ~0.5 OBS has little slack left before it starts skipping.
    pub fn frame_budget_ratio(&self) -> f64 {
        let target = self.video.target_fps();
        if target <= 0.0 {
            return 0.0;
        }
        let budget_ms = 1000.0 / target;
        (self.stats.average_frame_render_time / budget_ms).clamp(0.0, 1.0)
    }

    /// A one-word verdict combining drops, congestion and frame pacing.
    pub fn health(&self) -> Health {
        // Anything actually reaching the viewer as a dropped frame is the
        // worst case, so it wins outright.
        if self.stream.active && self.stream.drop_percent() >= 1.0 {
            return Health::Dropping;
        }
        if self.session_output_skip_percent() >= 1.0 {
            return Health::Dropping;
        }
        if self.stream.reconnecting {
            return Health::Dropping;
        }

        let congested = self.stream.active && self.stream.congestion >= 0.3;
        let starved = self.video.target_fps() > 0.0 && self.fps_ratio() < 0.95;
        let tight = self.frame_budget_ratio() >= 0.5;
        let skipping = self.session_render_skip_percent() >= 0.5;
        if congested || starved || tight || skipping {
            return Health::Strained;
        }

        Health::Healthy
    }

    /// Only the inputs worth showing in the audio panel.
    pub fn audio_inputs(&self) -> &[AudioInput] {
        &self.inputs
    }
}

/// Convert a dB fader position to a linear multiplier.
fn db_to_mul(db: f64) -> f64 {
    if db <= -100.0 {
        0.0
    } else {
        10_f64.powf(db / 20.0)
    }
}

/// Bits per second between two byte marks, updating the mark in place.
///
/// Returns 0.0 for the first sample (no previous mark), for a zero-length
/// interval, and when the counter goes backwards — which happens when an
/// output restarts between polls.
fn derive_bitrate(mark: &mut Option<ByteMark>, bytes: u64, now: Instant) -> f64 {
    let rate = match *mark {
        Some(previous) => {
            let elapsed = now.saturating_duration_since(previous.at).as_secs_f64();
            if elapsed <= 0.0 || bytes < previous.bytes {
                0.0
            } else {
                ((bytes - previous.bytes) as f64 * 8.0) / elapsed
            }
        }
        None => 0.0,
    };
    *mark = Some(ByteMark { bytes, at: now });
    rate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(stats: Stats, stream: StreamStatus, record: RecordStatus) -> Update {
        Update::Poll(Box::new(PollData {
            stats,
            stream,
            record,
        }))
    }

    fn streaming(bytes: u64) -> StreamStatus {
        StreamStatus {
            active: true,
            bytes,
            ..StreamStatus::default()
        }
    }

    fn snapshot_60fps() -> Snapshot {
        let mut snapshot = Snapshot::new(64);
        snapshot.video = VideoSettings {
            fps_numerator: 60,
            fps_denominator: 1,
            ..VideoSettings::default()
        };
        snapshot
    }

    #[test]
    fn a_fresh_snapshot_is_connecting_and_empty() {
        let snapshot = Snapshot::new(16);
        assert_eq!(
            snapshot.connection,
            ConnectionState::Connecting { attempt: 1 }
        );
        assert_eq!(snapshot.polls, 0);
        assert!(snapshot.last_poll.is_none());
        assert!(snapshot.log.is_empty());
        assert_eq!(snapshot.health(), Health::Healthy);
    }

    #[test]
    fn connection_labels_distinguish_first_attempt_from_retries() {
        assert_eq!(
            ConnectionState::Connecting { attempt: 1 }.label(),
            "CONNECTING"
        );
        assert_eq!(
            ConnectionState::Connecting { attempt: 4 }.label(),
            "RETRY #4"
        );
        assert_eq!(ConnectionState::Connected.label(), "CONNECTED");
        assert_eq!(
            ConnectionState::Fatal {
                reason: "x".into(),
                hint: None
            }
            .label(),
            "FAILED"
        );
    }

    #[test]
    fn polls_feed_the_histories() {
        let mut snapshot = Snapshot::new(8);
        let now = Instant::now();
        snapshot.apply(
            poll(
                Stats {
                    cpu_usage: 5.0,
                    active_fps: 60.0,
                    ..Stats::default()
                },
                StreamStatus::default(),
                RecordStatus::default(),
            ),
            now,
        );
        assert_eq!(snapshot.polls, 1);
        assert_eq!(snapshot.history.cpu.latest(), 5.0);
        assert_eq!(snapshot.history.fps.latest(), 60.0);
        assert_eq!(snapshot.age(now), Some(Duration::ZERO));
    }

    #[test]
    fn bitrate_is_derived_from_the_byte_delta() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();

        // First poll has no predecessor, so no rate yet.
        snapshot.apply(
            poll(Stats::default(), streaming(0), RecordStatus::default()),
            start,
        );
        assert_eq!(snapshot.stream_bitrate, 0.0);

        // 750_000 bytes over 1s = 6 Mbps.
        snapshot.apply(
            poll(
                Stats::default(),
                streaming(750_000),
                RecordStatus::default(),
            ),
            start + Duration::from_secs(1),
        );
        assert!((snapshot.stream_bitrate - 6_000_000.0).abs() < 1.0);
    }

    #[test]
    fn bitrate_ignores_a_counter_that_goes_backwards() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();
        snapshot.apply(
            poll(
                Stats::default(),
                streaming(1_000_000),
                RecordStatus::default(),
            ),
            start,
        );
        snapshot.apply(
            poll(Stats::default(), streaming(10), RecordStatus::default()),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.stream_bitrate, 0.0);
    }

    #[test]
    fn stopping_the_stream_zeroes_the_bitrate() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();
        snapshot.apply(
            poll(Stats::default(), streaming(0), RecordStatus::default()),
            start,
        );
        snapshot.apply(
            poll(
                Stats::default(),
                streaming(750_000),
                RecordStatus::default(),
            ),
            start + Duration::from_secs(1),
        );
        assert!(snapshot.stream_bitrate > 0.0);

        snapshot.apply(
            poll(
                Stats::default(),
                StreamStatus::default(),
                RecordStatus::default(),
            ),
            start + Duration::from_secs(2),
        );
        assert_eq!(snapshot.stream_bitrate, 0.0);
    }

    #[test]
    fn a_paused_recording_reports_no_write_rate() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();
        let recording = |bytes, paused| RecordStatus {
            active: true,
            paused,
            bytes,
            ..RecordStatus::default()
        };
        snapshot.apply(
            poll(
                Stats::default(),
                StreamStatus::default(),
                recording(0, false),
            ),
            start,
        );
        snapshot.apply(
            poll(
                Stats::default(),
                StreamStatus::default(),
                recording(500_000, true),
            ),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.record_bitrate, 0.0);
    }

    #[test]
    fn frame_counters_are_measured_from_the_start_of_the_stream() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();

        // OBS has been running a while and already skipped some frames.
        let before = Stats {
            render_skipped_frames: 500,
            render_total_frames: 100_000,
            output_skipped_frames: 200,
            output_total_frames: 90_000,
            ..Stats::default()
        };
        snapshot.apply(
            poll(
                before.clone(),
                StreamStatus::default(),
                RecordStatus::default(),
            ),
            start,
        );
        // No stream yet: fall back to the since-launch totals.
        assert_eq!(snapshot.session_render_skipped(), 500);

        // Stream starts — baseline captured here.
        snapshot.apply(
            poll(before, streaming(0), RecordStatus::default()),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.session_render_skipped(), 0);
        assert_eq!(snapshot.session_output_skipped(), 0);

        // Ten more render skips during the stream.
        let during = Stats {
            render_skipped_frames: 510,
            render_total_frames: 101_000,
            output_skipped_frames: 205,
            output_total_frames: 90_500,
            ..Stats::default()
        };
        snapshot.apply(
            poll(during, streaming(100), RecordStatus::default()),
            start + Duration::from_secs(2),
        );
        assert_eq!(snapshot.session_render_skipped(), 10);
        assert_eq!(snapshot.session_output_skipped(), 5);
        assert!((snapshot.session_render_skip_percent() - 1.0).abs() < 1e-9);
        assert!((snapshot.session_output_skip_percent() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn stopping_the_stream_restores_since_launch_counters() {
        let mut snapshot = Snapshot::new(8);
        let start = Instant::now();
        let stats = Stats {
            render_skipped_frames: 42,
            render_total_frames: 1000,
            ..Stats::default()
        };
        snapshot.apply(
            poll(stats.clone(), streaming(0), RecordStatus::default()),
            start,
        );
        assert_eq!(snapshot.session_render_skipped(), 0);

        snapshot.apply(
            poll(stats, StreamStatus::default(), RecordStatus::default()),
            start + Duration::from_secs(1),
        );
        assert_eq!(snapshot.session_render_skipped(), 42);
    }

    #[test]
    fn fps_ratio_is_relative_to_the_configured_target() {
        let mut snapshot = snapshot_60fps();
        snapshot.stats.active_fps = 30.0;
        assert!((snapshot.fps_ratio() - 0.5).abs() < 1e-9);

        snapshot.stats.active_fps = 90.0;
        assert_eq!(snapshot.fps_ratio(), 1.0, "overshoot clamps to full");

        // Before video settings arrive, assume healthy rather than alarming.
        assert_eq!(Snapshot::new(8).fps_ratio(), 1.0);
    }

    #[test]
    fn frame_budget_ratio_compares_render_time_to_the_frame_interval() {
        let mut snapshot = snapshot_60fps();
        // 60fps → 16.67ms budget; 8.33ms of render is half of it.
        snapshot.stats.average_frame_render_time = 1000.0 / 60.0 / 2.0;
        assert!((snapshot.frame_budget_ratio() - 0.5).abs() < 1e-6);

        snapshot.stats.average_frame_render_time = 100.0;
        assert_eq!(snapshot.frame_budget_ratio(), 1.0);
        assert_eq!(Snapshot::new(8).frame_budget_ratio(), 0.0);
    }

    #[test]
    fn health_reports_dropping_when_frames_are_lost() {
        let mut snapshot = snapshot_60fps();
        snapshot.stats.active_fps = 60.0;
        snapshot.stream = StreamStatus {
            active: true,
            skipped_frames: 30,
            total_frames: 1000,
            ..StreamStatus::default()
        };
        assert_eq!(snapshot.health(), Health::Dropping);
    }

    #[test]
    fn health_reports_dropping_while_reconnecting() {
        let mut snapshot = snapshot_60fps();
        snapshot.stats.active_fps = 60.0;
        snapshot.stream = StreamStatus {
            active: true,
            reconnecting: true,
            ..StreamStatus::default()
        };
        assert_eq!(snapshot.health(), Health::Dropping);
    }

    #[test]
    fn health_reports_strained_on_congestion_or_starved_fps() {
        let mut congested = snapshot_60fps();
        congested.stats.active_fps = 60.0;
        congested.stream = StreamStatus {
            active: true,
            congestion: 0.5,
            ..StreamStatus::default()
        };
        assert_eq!(congested.health(), Health::Strained);

        let mut starved = snapshot_60fps();
        starved.stats.active_fps = 45.0;
        assert_eq!(starved.health(), Health::Strained);
    }

    #[test]
    fn health_is_healthy_when_everything_is_within_budget() {
        let mut snapshot = snapshot_60fps();
        snapshot.stats.active_fps = 60.0;
        snapshot.stats.average_frame_render_time = 2.0;
        snapshot.stream = StreamStatus {
            active: true,
            congestion: 0.01,
            skipped_frames: 0,
            total_frames: 5000,
            ..StreamStatus::default()
        };
        assert_eq!(snapshot.health(), Health::Healthy);
        assert_eq!(Health::Healthy.label(), "HEALTHY");
    }

    #[test]
    fn inventory_merges_rather_than_clobbering_live_levels() {
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        snapshot.apply_meters(&[MeterFrame {
            name: "Mic".to_string(),
            peaks: vec![0.4, 0.5],
        }]);
        assert_eq!(snapshot.inputs[0].peaks, vec![0.4, 0.5]);

        // A second inventory poll must not blank the meter.
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        assert_eq!(snapshot.inputs[0].peaks, vec![0.4, 0.5]);
    }

    #[test]
    fn inventory_drops_inputs_that_no_longer_exist() {
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_inventory(Inventory {
            inputs: vec![
                AudioInput {
                    name: "Mic".into(),
                    ..AudioInput::default()
                },
                AudioInput {
                    name: "Desktop".into(),
                    ..AudioInput::default()
                },
            ],
            ..Inventory::default()
        });
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        assert_eq!(snapshot.inputs.len(), 1);
        assert_eq!(snapshot.inputs[0].name, "Mic");
    }

    #[test]
    fn faders_and_meters_update_separate_fields() {
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });

        snapshot.apply_faders(&[FaderState {
            name: "Mic".to_string(),
            muted: Some(true),
            volume_db: Some(-6.0),
        }]);
        assert!(snapshot.inputs[0].muted);
        assert!((snapshot.inputs[0].volume_db + 6.0).abs() < 1e-6);
        assert!(
            snapshot.inputs[0].peaks.is_empty(),
            "a fader position is not a peak frame"
        );

        snapshot.apply_meters(&[MeterFrame {
            name: "Mic".to_string(),
            peaks: vec![0.3, 0.6],
        }]);
        assert_eq!(snapshot.inputs[0].peaks, vec![0.3, 0.6]);
        assert!(snapshot.inputs[0].muted, "mute state survives a peak frame");
    }

    #[test]
    fn a_silent_stereo_meter_frame_does_not_unmute_the_input() {
        // Regression: peaks and fader positions used to share one channel and
        // were told apart by their values, so digital silence on two channels
        // — [0.0, 0.0] — was read as "unmuted, 0 dB" and wiped the real state.
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        snapshot.apply_faders(&[FaderState {
            name: "Mic".to_string(),
            muted: Some(true),
            volume_db: Some(-12.0),
        }]);

        snapshot.apply_meters(&[MeterFrame {
            name: "Mic".to_string(),
            peaks: vec![0.0, 0.0],
        }]);

        assert!(snapshot.inputs[0].muted, "silence must not unmute");
        assert!((snapshot.inputs[0].volume_db + 12.0).abs() < 1e-6);
        assert_eq!(snapshot.inputs[0].peaks, vec![0.0, 0.0]);
    }

    #[test]
    fn a_fader_reply_missing_one_half_leaves_the_other_alone() {
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        snapshot.apply_faders(&[FaderState {
            name: "Mic".to_string(),
            muted: Some(true),
            volume_db: Some(-6.0),
        }]);

        // Only the volume half answered this time.
        snapshot.apply_faders(&[FaderState {
            name: "Mic".to_string(),
            muted: None,
            volume_db: Some(-3.0),
        }]);

        assert!(
            snapshot.inputs[0].muted,
            "an unanswered half keeps its value"
        );
        assert!((snapshot.inputs[0].volume_db + 3.0).abs() < 1e-6);
    }

    #[test]
    fn meters_for_unknown_inputs_are_ignored() {
        let mut snapshot = Snapshot::new(8);
        snapshot.apply_meters(&[MeterFrame {
            name: "Ghost".to_string(),
            peaks: vec![0.5],
        }]);
        assert!(snapshot.inputs.is_empty());
    }

    #[test]
    fn losing_the_connection_silences_the_meters_and_rates() {
        let mut snapshot = Snapshot::new(8);
        let now = Instant::now();
        snapshot.apply(Update::Connected(Box::default()), now);
        snapshot.apply_inventory(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            }],
            ..Inventory::default()
        });
        snapshot.apply_meters(&[MeterFrame {
            name: "Mic".to_string(),
            peaks: vec![0.9],
        }]);
        snapshot.stream_bitrate = 6_000_000.0;

        snapshot.apply(
            Update::Disconnected {
                reason: "closed".into(),
                hint: None,
                retry_in: Duration::from_secs(1),
            },
            now,
        );
        assert!(snapshot.inputs[0].peaks.is_empty());
        assert_eq!(snapshot.stream_bitrate, 0.0);
        assert!(matches!(
            snapshot.connection,
            ConnectionState::Disconnected { .. }
        ));
    }

    #[test]
    fn the_version_banner_is_not_blanked_by_a_later_partial_update() {
        let mut snapshot = Snapshot::new(8);
        let now = Instant::now();
        snapshot.apply(
            Update::Connected(Box::new(Version {
                websocket_version: "5.5.0".into(),
                ..Version::default()
            })),
            now,
        );
        snapshot.apply(
            Update::Connected(Box::new(Version {
                obs_version: "30.2.3".into(),
                platform: "linux".into(),
                websocket_version: String::new(),
            })),
            now,
        );
        assert_eq!(snapshot.version.obs_version, "30.2.3");
        assert_eq!(snapshot.version.websocket_version, "5.5.0");
    }

    #[test]
    fn sustained_network_loss_raises_an_alert_and_logs_it() {
        use crate::metrics::alerts::AlertKind;

        let mut snapshot = Snapshot::new(32);
        let start = Instant::now();
        let dropping = |dropped| StreamStatus {
            active: true,
            skipped_frames: dropped,
            total_frames: 1000,
            ..StreamStatus::default()
        };

        // Baseline, then a clean poll: nothing to say.
        assert!(
            snapshot
                .apply(
                    poll(Stats::default(), dropping(0), RecordStatus::default()),
                    start
                )
                .is_empty()
        );
        assert!(
            snapshot
                .apply(
                    poll(Stats::default(), dropping(0), RecordStatus::default()),
                    start + Duration::from_secs(1)
                )
                .is_empty()
        );

        let alerts = snapshot.apply(
            poll(Stats::default(), dropping(9), RecordStatus::default()),
            start + Duration::from_secs(2),
        );
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::NetworkDropped);
        assert_eq!(alerts[0].frames, 9);

        // And it is durable: the Log view still shows it after the banner goes.
        let logged = snapshot.log.last().expect("alert should be logged");
        assert_eq!(logged.level, LogLevel::Error);
        assert!(
            logged.message.contains("dropping frames"),
            "{}",
            logged.message
        );
    }

    #[test]
    fn an_idle_obs_never_raises_frame_alerts() {
        let mut snapshot = Snapshot::new(32);
        let start = Instant::now();
        let idle = |renderer| {
            poll(
                Stats {
                    render_skipped_frames: renderer,
                    ..Stats::default()
                },
                StreamStatus::default(),
                RecordStatus::default(),
            )
        };
        snapshot.apply(idle(0), start);
        assert!(
            snapshot
                .apply(idle(500), start + Duration::from_secs(1))
                .is_empty()
        );
    }

    #[test]
    fn alert_thresholds_can_be_reconfigured() {
        let mut snapshot = Snapshot::new(32);
        snapshot.set_alert_config(crate::metrics::AlertConfig {
            enabled: false,
            ..crate::metrics::AlertConfig::default()
        });

        let start = Instant::now();
        let dropping = |dropped| StreamStatus {
            active: true,
            skipped_frames: dropped,
            ..StreamStatus::default()
        };
        snapshot.apply(
            poll(Stats::default(), dropping(0), RecordStatus::default()),
            start,
        );
        assert!(
            snapshot
                .apply(
                    poll(Stats::default(), dropping(100), RecordStatus::default()),
                    start + Duration::from_secs(1)
                )
                .is_empty()
        );
    }

    #[test]
    fn the_log_is_capped_and_keeps_the_newest_lines() {
        let mut snapshot = Snapshot::new(8);
        let now = Instant::now();
        for index in 0..(LOG_CAPACITY + 10) {
            snapshot.push_log(LogLevel::Info, format!("line {index}"), now);
        }
        assert_eq!(snapshot.log.len(), LOG_CAPACITY);
        assert_eq!(snapshot.log[0].message, "line 10");
        assert_eq!(
            snapshot.log.last().unwrap().message,
            format!("line {}", LOG_CAPACITY + 9)
        );
    }

    #[test]
    fn reconnecting_logs_only_on_the_transition_out_of_connected() {
        let mut snapshot = Snapshot::new(8);
        let now = Instant::now();
        let drop = || Update::Disconnected {
            reason: "closed".into(),
            hint: None,
            retry_in: Duration::from_secs(1),
        };

        snapshot.apply(Update::Connected(Box::default()), now);
        snapshot.apply(drop(), now);
        let after_first = snapshot.log.len();

        // Repeated failures while already offline must not spam the log.
        snapshot.apply(drop(), now);
        assert_eq!(snapshot.log.len(), after_first);
    }

    #[test]
    fn history_capacity_can_be_resized_at_runtime() {
        let mut snapshot = Snapshot::new(4);
        let start = Instant::now();
        for index in 0..6 {
            snapshot.apply(
                poll(
                    Stats {
                        cpu_usage: index as f64,
                        ..Stats::default()
                    },
                    StreamStatus::default(),
                    RecordStatus::default(),
                ),
                start + Duration::from_secs(index),
            );
        }
        assert_eq!(snapshot.history.cpu.len(), 4);
        snapshot.set_history_capacity(2);
        assert_eq!(snapshot.history.cpu.len(), 2);
        assert_eq!(snapshot.history.cpu.latest(), 5.0);
    }

    #[test]
    fn db_to_mul_matches_known_fader_positions() {
        assert!((db_to_mul(0.0) - 1.0).abs() < 1e-9);
        assert!((db_to_mul(-6.0) - 0.501).abs() < 1e-3);
        assert_eq!(db_to_mul(-100.0), 0.0);
        assert_eq!(db_to_mul(f64::NEG_INFINITY), 0.0);
    }
}

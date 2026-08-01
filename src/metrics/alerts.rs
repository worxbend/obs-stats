//! Detecting that OBS is *currently* losing frames.
//!
//! The dashboard's counters answer "how many frames have been lost?". An
//! alert has to answer a different question — "is something going wrong right
//! now?" — and those come apart badly. A stream that dropped forty frames an
//! hour ago and none since is healthy; one that has dropped three in the last
//! five seconds is not, even though its total is smaller.
//!
//! So this watches *deltas* over a rolling window rather than totals, and
//! rate-limits per cause. Everything here is pure: it takes counters and a
//! timestamp and returns alerts, which makes the thresholds testable without
//! a stream, a terminal, or a clock.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Which pipeline is losing frames.
///
/// Kept separate because the causes are unrelated and so are the fixes —
/// telling someone "OBS dropped frames" when their GPU is the problem sends
/// them to look at their internet connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// The renderer could not build frames in time. A GPU problem.
    RendererSkipped,
    /// The encoder could not keep up. A CPU or encoder-settings problem.
    EncoderSkipped,
    /// Frames were encoded but not delivered. An uplink problem.
    NetworkDropped,
    /// The stream dropped and OBS is trying to re-establish it.
    Reconnecting,
}

impl AlertKind {
    /// Every kind, in the order they are checked.
    pub const ALL: [AlertKind; 4] = [
        AlertKind::NetworkDropped,
        AlertKind::EncoderSkipped,
        AlertKind::RendererSkipped,
        AlertKind::Reconnecting,
    ];

    /// Stable slot for the per-kind window and cooldown bookkeeping.
    pub fn index(self) -> usize {
        match self {
            AlertKind::NetworkDropped => 0,
            AlertKind::EncoderSkipped => 1,
            AlertKind::RendererSkipped => 2,
            AlertKind::Reconnecting => 3,
        }
    }

    /// Short label for the TUI banner.
    pub fn label(self) -> &'static str {
        match self {
            AlertKind::RendererSkipped => "RENDER LAG",
            AlertKind::EncoderSkipped => "ENCODER LAG",
            AlertKind::NetworkDropped => "DROPPING FRAMES",
            AlertKind::Reconnecting => "RECONNECTING",
        }
    }

    /// What the viewer actually experiences, for the notification body.
    pub fn consequence(self) -> &'static str {
        match self {
            AlertKind::RendererSkipped => "the canvas is missing frames",
            AlertKind::EncoderSkipped => "the encoder is falling behind",
            AlertKind::NetworkDropped => "viewers are seeing stutter",
            AlertKind::Reconnecting => "the stream has dropped",
        }
    }

    /// The one-line thing to go and do about it.
    ///
    /// Deliberately short: it shares the banner row with the cause and the
    /// count, and `consequence` has already said what is wrong, so this only
    /// needs to say what to change.
    pub fn hint(self) -> &'static str {
        match self {
            AlertKind::RendererSkipped => "Lower the canvas resolution, FPS, or filter load",
            AlertKind::EncoderSkipped => "Use a faster preset, or a hardware encoder",
            AlertKind::NetworkDropped => "Lower the bitrate, or enable dynamic bitrate",
            AlertKind::Reconnecting => "Check the network path to your ingest server",
        }
    }

    /// Network loss is the only one a viewer sees directly, so it outranks the
    /// others; a dropped stream outranks everything.
    pub fn severity(self) -> Severity {
        match self {
            AlertKind::Reconnecting | AlertKind::NetworkDropped => Severity::Critical,
            AlertKind::EncoderSkipped | AlertKind::RendererSkipped => Severity::Warning,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Critical,
}

/// One raised alert.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    /// Frames lost within the detection window. Zero for [`AlertKind::Reconnecting`],
    /// which is a state change rather than a count.
    pub frames: u64,
    /// The window the count was measured over.
    pub window: Duration,
    pub at: Instant,
}

impl Alert {
    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }

    /// Notification title.
    pub fn title(&self) -> String {
        format!("OBS: {}", self.kind.label().to_ascii_lowercase())
    }

    /// One-line summary for the TUI banner and the notification body.
    pub fn summary(&self) -> String {
        match self.kind {
            AlertKind::Reconnecting => self.kind.consequence().to_string(),
            _ => format!(
                "{} {} in {}s — {}",
                self.frames,
                if self.frames == 1 { "frame" } else { "frames" },
                self.window.as_secs().max(1),
                self.kind.consequence()
            ),
        }
    }
}

/// Thresholds governing when an alert fires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlertConfig {
    pub enabled: bool,
    /// Frames that must be lost within the window before alerting.
    ///
    /// One lost frame in a 60fps stream is invisible and constant on some
    /// systems; alerting on it would train the user to ignore the alert.
    pub min_frames: u64,
    /// How far back to count losses.
    pub window: Duration,
    /// Minimum gap between two alerts of the same kind.
    pub cooldown: Duration,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_frames: 3,
            window: Duration::from_secs(10),
            cooldown: Duration::from_secs(60),
        }
    }
}

/// The counters an observation is derived from.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Counters {
    /// Cumulative frames skipped by the renderer.
    pub render_skipped: u64,
    /// Cumulative frames skipped by the encoder.
    pub output_skipped: u64,
    /// Cumulative frames dropped by the network, per stream session.
    pub network_dropped: u64,
    /// Whether any output is running. Renderer and encoder losses are only
    /// worth interrupting someone about when something is being produced.
    pub outputting: bool,
    /// Whether the stream output is currently reconnecting.
    pub reconnecting: bool,
}

/// Rolling loss detection with per-kind rate limiting.
#[derive(Debug, Clone)]
pub struct Watcher {
    config: AlertConfig,
    previous: Option<Counters>,
    /// Per kind: `(timestamp, frames lost)` samples inside the window.
    windows: [VecDeque<(Instant, u64)>; 4],
    last_fired: [Option<Instant>; 4],
    was_reconnecting: bool,
}

impl Watcher {
    pub fn new(config: AlertConfig) -> Self {
        Self {
            config,
            previous: None,
            windows: Default::default(),
            last_fired: [None; 4],
            was_reconnecting: false,
        }
    }

    pub fn config(&self) -> AlertConfig {
        self.config
    }

    pub fn set_config(&mut self, config: AlertConfig) {
        self.config = config;
    }

    /// Fold in one poll's counters and return any alerts it raised.
    ///
    /// The first observation only establishes a baseline: without a previous
    /// reading every cumulative counter would look like a fresh loss, and
    /// starting the app during a healthy stream would alert immediately.
    pub fn observe(&mut self, counters: Counters, now: Instant) -> Vec<Alert> {
        let Some(previous) = self.previous.replace(counters) else {
            self.was_reconnecting = counters.reconnecting;
            return Vec::new();
        };

        if !self.config.enabled {
            self.was_reconnecting = counters.reconnecting;
            return Vec::new();
        }

        let mut alerts = Vec::new();

        // A stream restarting resets these counters; `delta` treats any
        // backwards step as zero rather than as an enormous loss.
        if counters.outputting {
            for (kind, current, before) in [
                (
                    AlertKind::NetworkDropped,
                    counters.network_dropped,
                    previous.network_dropped,
                ),
                (
                    AlertKind::EncoderSkipped,
                    counters.output_skipped,
                    previous.output_skipped,
                ),
                (
                    AlertKind::RendererSkipped,
                    counters.render_skipped,
                    previous.render_skipped,
                ),
            ] {
                if let Some(alert) = self.record(kind, delta(current, before), now) {
                    alerts.push(alert);
                }
            }
        } else {
            // Nothing is being produced, so keep the windows from carrying
            // stale losses into the next session.
            for kind in [
                AlertKind::NetworkDropped,
                AlertKind::EncoderSkipped,
                AlertKind::RendererSkipped,
            ] {
                self.windows[kind.index()].clear();
            }
        }

        // Reconnection is a state transition, not a count: alert on the edge.
        if counters.reconnecting && !self.was_reconnecting {
            alerts.push(Alert {
                kind: AlertKind::Reconnecting,
                frames: 0,
                window: self.config.window,
                at: now,
            });
            self.last_fired[AlertKind::Reconnecting.index()] = Some(now);
        }
        self.was_reconnecting = counters.reconnecting;

        alerts
    }

    /// Add `lost` frames to `kind`'s window and fire if it crosses threshold.
    fn record(&mut self, kind: AlertKind, lost: u64, now: Instant) -> Option<Alert> {
        let slot = kind.index();
        let window = &mut self.windows[slot];

        if lost > 0 {
            window.push_back((now, lost));
        }
        // Evict samples that have aged out of the window.
        while let Some((at, _)) = window.front() {
            if now.saturating_duration_since(*at) > self.config.window {
                window.pop_front();
            } else {
                break;
            }
        }

        let total: u64 = window.iter().map(|(_, lost)| *lost).sum();
        if total < self.config.min_frames.max(1) {
            return None;
        }

        if let Some(fired) = self.last_fired[slot]
            && now.saturating_duration_since(fired) < self.config.cooldown
        {
            return None;
        }

        self.last_fired[slot] = Some(now);
        // Start a fresh window so the next alert reports newly lost frames
        // rather than re-reporting the ones just announced.
        window.clear();

        Some(Alert {
            kind,
            frames: total,
            window: self.config.window,
            at: now,
        })
    }
}

/// Difference between two readings of a cumulative counter.
///
/// Returns zero when the counter went backwards, which happens whenever an
/// output restarts and its per-session counters reset.
fn delta(current: u64, previous: u64) -> u64 {
    current.saturating_sub(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AlertConfig {
        AlertConfig {
            enabled: true,
            min_frames: 3,
            window: Duration::from_secs(10),
            cooldown: Duration::from_secs(60),
        }
    }

    fn streaming(network: u64, encoder: u64, renderer: u64) -> Counters {
        Counters {
            network_dropped: network,
            output_skipped: encoder,
            render_skipped: renderer,
            outputting: true,
            reconnecting: false,
        }
    }

    #[test]
    fn the_first_observation_only_establishes_a_baseline() {
        // Otherwise starting the app mid-stream would alert on history.
        let mut watcher = Watcher::new(config());
        let alerts = watcher.observe(streaming(500, 300, 900), Instant::now());
        assert!(alerts.is_empty());
    }

    #[test]
    fn a_loss_below_the_threshold_does_not_alert() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);
        let alerts = watcher.observe(streaming(2, 0, 0), start + Duration::from_secs(1));
        assert!(alerts.is_empty(), "2 frames is under the 3-frame threshold");
    }

    #[test]
    fn crossing_the_threshold_alerts_once() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        let alerts = watcher.observe(streaming(3, 0, 0), start + Duration::from_secs(1));
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::NetworkDropped);
        assert_eq!(alerts[0].frames, 3);
        assert_eq!(alerts[0].severity(), Severity::Critical);
    }

    #[test]
    fn losses_accumulate_across_polls_within_the_window() {
        // One frame per poll never crosses the threshold on its own, but three
        // polls running is exactly the sustained loss worth reporting.
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        assert!(
            watcher
                .observe(streaming(1, 0, 0), start + Duration::from_secs(1))
                .is_empty()
        );
        assert!(
            watcher
                .observe(streaming(2, 0, 0), start + Duration::from_secs(2))
                .is_empty()
        );
        let alerts = watcher.observe(streaming(3, 0, 0), start + Duration::from_secs(3));
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].frames, 3);
    }

    #[test]
    fn losses_spread_wider_than_the_window_never_accumulate() {
        // One frame every 30s on a 10s window is background noise, not a fault.
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        for step in 1..=6 {
            let alerts = watcher.observe(
                streaming(step, 0, 0),
                start + Duration::from_secs(step * 30),
            );
            assert!(alerts.is_empty(), "step {step} should not alert");
        }
    }

    #[test]
    fn the_cooldown_suppresses_repeat_alerts_of_the_same_kind() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        assert_eq!(
            watcher
                .observe(streaming(5, 0, 0), start + Duration::from_secs(1))
                .len(),
            1
        );
        // Still dropping, but within the 60s cooldown.
        assert!(
            watcher
                .observe(streaming(20, 0, 0), start + Duration::from_secs(5))
                .is_empty()
        );
        assert!(
            watcher
                .observe(streaming(40, 0, 0), start + Duration::from_secs(30))
                .is_empty()
        );
        // Past the cooldown, it may speak again.
        assert_eq!(
            watcher
                .observe(streaming(60, 0, 0), start + Duration::from_secs(70))
                .len(),
            1
        );
    }

    #[test]
    fn an_alert_reports_only_newly_lost_frames() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        let first = watcher.observe(streaming(5, 0, 0), start + Duration::from_secs(1));
        assert_eq!(first[0].frames, 5);

        // 10 more after the cooldown: the alert must say 10, not 15.
        let second = watcher.observe(streaming(15, 0, 0), start + Duration::from_secs(70));
        assert_eq!(second[0].frames, 10);
    }

    #[test]
    fn each_pipeline_alerts_independently() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);

        let alerts = watcher.observe(streaming(4, 5, 6), start + Duration::from_secs(1));
        let kinds: Vec<AlertKind> = alerts.iter().map(|alert| alert.kind).collect();
        assert_eq!(
            kinds,
            [
                AlertKind::NetworkDropped,
                AlertKind::EncoderSkipped,
                AlertKind::RendererSkipped
            ],
            "most viewer-visible cause first"
        );
    }

    #[test]
    fn a_counter_reset_is_not_a_loss() {
        // Restarting a stream resets its per-session counters to zero.
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(900, 0, 0), start);
        let alerts = watcher.observe(streaming(0, 0, 0), start + Duration::from_secs(1));
        assert!(alerts.is_empty());
    }

    #[test]
    fn nothing_alerts_while_no_output_is_running() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        let idle = |renderer| Counters {
            render_skipped: renderer,
            outputting: false,
            ..Counters::default()
        };
        watcher.observe(idle(0), start);
        // The renderer can skip while idle; nobody is watching, so stay quiet.
        assert!(
            watcher
                .observe(idle(50), start + Duration::from_secs(1))
                .is_empty()
        );
    }

    #[test]
    fn going_idle_clears_the_window_so_losses_do_not_carry_over() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);
        watcher.observe(streaming(2, 0, 0), start + Duration::from_secs(1));

        // Stream stops, then a new one starts within the window.
        watcher.observe(
            Counters {
                outputting: false,
                ..Counters::default()
            },
            start + Duration::from_secs(2),
        );
        // Two frames from the previous session must not count towards this one.
        let alerts = watcher.observe(streaming(2, 0, 0), start + Duration::from_secs(3));
        assert!(alerts.is_empty());
    }

    #[test]
    fn reconnecting_alerts_on_the_transition_only() {
        let mut watcher = Watcher::new(config());
        let start = Instant::now();
        let state = |reconnecting| Counters {
            outputting: true,
            reconnecting,
            ..Counters::default()
        };
        watcher.observe(state(false), start);

        let alerts = watcher.observe(state(true), start + Duration::from_secs(1));
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::Reconnecting);
        assert_eq!(alerts[0].severity(), Severity::Critical);

        // Still reconnecting on the next poll: do not repeat.
        assert!(
            watcher
                .observe(state(true), start + Duration::from_secs(2))
                .is_empty()
        );

        // Recovers, then drops again: that is a new event.
        watcher.observe(state(false), start + Duration::from_secs(3));
        assert_eq!(
            watcher
                .observe(state(true), start + Duration::from_secs(4))
                .len(),
            1
        );
    }

    #[test]
    fn disabling_the_watcher_silences_everything() {
        let mut watcher = Watcher::new(AlertConfig {
            enabled: false,
            ..config()
        });
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);
        assert!(
            watcher
                .observe(streaming(500, 500, 500), start + Duration::from_secs(1))
                .is_empty()
        );
    }

    #[test]
    fn a_zero_threshold_still_needs_one_lost_frame() {
        // Guards against a config of 0 alerting on every single poll.
        let mut watcher = Watcher::new(AlertConfig {
            min_frames: 0,
            ..config()
        });
        let start = Instant::now();
        watcher.observe(streaming(0, 0, 0), start);
        assert!(
            watcher
                .observe(streaming(0, 0, 0), start + Duration::from_secs(1))
                .is_empty()
        );
        assert_eq!(
            watcher
                .observe(streaming(1, 0, 0), start + Duration::from_secs(2))
                .len(),
            1
        );
    }

    #[test]
    fn summaries_read_as_sentences() {
        let alert = Alert {
            kind: AlertKind::NetworkDropped,
            frames: 12,
            window: Duration::from_secs(10),
            at: Instant::now(),
        };
        assert_eq!(
            alert.summary(),
            "12 frames in 10s — viewers are seeing stutter"
        );
        assert_eq!(alert.title(), "OBS: dropping frames");

        let single = Alert {
            frames: 1,
            ..alert.clone()
        };
        assert!(
            single.summary().starts_with("1 frame in"),
            "{}",
            single.summary()
        );

        let reconnect = Alert {
            kind: AlertKind::Reconnecting,
            frames: 0,
            ..alert
        };
        assert_eq!(reconnect.summary(), "the stream has dropped");
    }

    #[test]
    fn every_kind_is_fully_described_and_uniquely_slotted() {
        use std::collections::HashSet;
        let slots: HashSet<usize> = AlertKind::ALL.iter().map(|kind| kind.index()).collect();
        assert_eq!(slots.len(), AlertKind::ALL.len(), "duplicate slot");
        assert!(slots.iter().all(|slot| *slot < 4), "slot out of range");

        for kind in AlertKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(!kind.consequence().is_empty());
            assert!(!kind.hint().is_empty());
            // The banner shares one row between cause, count and hint.
            assert!(
                kind.hint().chars().count() <= 50,
                "{kind:?} hint is too long for the banner row: {}",
                kind.hint().chars().count()
            );
        }
    }

    #[test]
    fn network_loss_outranks_renderer_loss() {
        assert!(AlertKind::NetworkDropped.severity() > AlertKind::RendererSkipped.severity());
        assert_eq!(AlertKind::Reconnecting.severity(), Severity::Critical);
    }

    #[test]
    fn delta_ignores_a_counter_going_backwards() {
        assert_eq!(delta(10, 4), 6);
        assert_eq!(delta(4, 10), 0);
        assert_eq!(delta(0, 0), 0);
    }
}

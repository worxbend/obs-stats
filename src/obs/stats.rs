//! Typed views over obs-websocket response payloads.
//!
//! Every decoder is total: a missing or wrong-typed field falls back to its
//! default rather than failing the poll. OBS adds fields between point
//! releases and drops others when an output is inactive, and a dashboard that
//! blanks out because one number moved is worse than one showing a zero.

use serde_json::Value;

/// `GetStats` — process-wide health counters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    /// CPU usage of the OBS process, as a percentage of one core-equivalent.
    pub cpu_usage: f64,
    /// Resident memory, in MiB.
    pub memory_usage: f64,
    /// Free space on the recording volume, in MiB.
    pub available_disk_space: f64,
    /// Frames per second the renderer is currently achieving.
    pub active_fps: f64,
    /// Mean render time per frame, in milliseconds.
    pub average_frame_render_time: f64,
    pub render_skipped_frames: u64,
    pub render_total_frames: u64,
    pub output_skipped_frames: u64,
    pub output_total_frames: u64,
    pub websocket_incoming_messages: u64,
    pub websocket_outgoing_messages: u64,
}

impl Stats {
    pub fn from_value(value: &Value) -> Self {
        Self {
            cpu_usage: number(value, "cpuUsage"),
            memory_usage: number(value, "memoryUsage"),
            available_disk_space: number(value, "availableDiskSpace"),
            active_fps: number(value, "activeFps"),
            average_frame_render_time: number(value, "averageFrameRenderTime"),
            render_skipped_frames: count(value, "renderSkippedFrames"),
            render_total_frames: count(value, "renderTotalFrames"),
            output_skipped_frames: count(value, "outputSkippedFrames"),
            output_total_frames: count(value, "outputTotalFrames"),
            websocket_incoming_messages: count(value, "webSocketSessionIncomingMessages"),
            websocket_outgoing_messages: count(value, "webSocketSessionOutgoingMessages"),
        }
    }

    /// The renderer's skipped/total pair, since OBS launched.
    pub fn render(&self) -> FrameTally {
        FrameTally {
            skipped: self.render_skipped_frames,
            total: self.render_total_frames,
        }
    }

    /// The encoder's skipped/total pair, since OBS launched.
    pub fn output(&self) -> FrameTally {
        FrameTally {
            skipped: self.output_skipped_frames,
            total: self.output_total_frames,
        }
    }

    /// Share of rendered frames that were skipped, since OBS launched.
    pub fn render_skip_percent(&self) -> f64 {
        self.render().percent()
    }

    /// Share of encoded frames that were skipped, since OBS launched.
    pub fn output_skip_percent(&self) -> f64 {
        self.output().percent()
    }

    /// Resident memory in bytes, for formatters that want a unit-suffixed
    /// figure rather than a bare MiB number.
    pub fn memory_bytes(&self) -> f64 {
        self.memory_usage * MIB
    }

    /// Free space on the recording volume, in bytes.
    ///
    /// Stays `f64` rather than rounding to `u64`: the recording-time-left
    /// estimate divides this by a bitrate, and truncating first would throw
    /// away precision before the division that needs it.
    pub fn available_disk_bytes(&self) -> f64 {
        self.available_disk_space * MIB
    }
}

/// Bytes in a mebibyte. OBS reports memory and disk space in MiB; anything
/// that formats those with a unit suffix has to convert first.
pub const MIB: f64 = 1024.0 * 1024.0;

/// Free disk space, in MiB, below which recording is at risk.
///
/// Lives here beside the field it qualifies, so the stream view and the
/// overview cannot colour the same number against different thresholds.
pub const DISK_WARN_MIB: f64 = 20_000.0;
pub const DISK_DANGER_MIB: f64 = 5_000.0;

/// A skipped-out-of-total frame count.
///
/// The two numbers are meaningless apart — a count of 900 skipped frames is
/// a catastrophe out of 1,000 and a rounding error out of 10,000,000 — so
/// they travel together and the rate is derived rather than stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameTally {
    pub skipped: u64,
    pub total: u64,
}

impl FrameTally {
    /// Skipped frames as a percentage of the total. Zero total reads as 0.0
    /// rather than dividing by zero: before the first frame there is nothing
    /// to be worried about yet.
    pub fn percent(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.skipped as f64 / self.total as f64) * 100.0
        }
    }

    /// This tally measured from `baseline` instead of from OBS launch.
    ///
    /// Saturating on purpose: a counter that went backwards means the output
    /// restarted, and that must read as "nothing yet this session" rather
    /// than underflowing into an enormous number.
    pub fn since(self, baseline: FrameTally) -> FrameTally {
        FrameTally {
            skipped: self.skipped.saturating_sub(baseline.skipped),
            total: self.total.saturating_sub(baseline.total),
        }
    }
}

/// `GetStreamStatus` — live streaming output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamStatus {
    pub active: bool,
    pub reconnecting: bool,
    pub timecode: String,
    pub duration_ms: u64,
    /// Encoder backpressure, `0.0` (healthy) to `1.0` (saturated).
    pub congestion: f64,
    pub bytes: u64,
    pub skipped_frames: u64,
    pub total_frames: u64,
}

impl StreamStatus {
    pub fn from_value(value: &Value) -> Self {
        Self {
            active: flag(value, "outputActive"),
            reconnecting: flag(value, "outputReconnecting"),
            timecode: text(value, "outputTimecode"),
            duration_ms: count(value, "outputDuration"),
            congestion: number(value, "outputCongestion"),
            bytes: count(value, "outputBytes"),
            skipped_frames: count(value, "outputSkippedFrames"),
            total_frames: count(value, "outputTotalFrames"),
        }
    }

    /// Frames the network dropped out of frames sent, since the stream
    /// started. Unlike the counters in [`Stats`], these are already scoped to
    /// the current stream by OBS.
    pub fn dropped(&self) -> FrameTally {
        FrameTally {
            skipped: self.skipped_frames,
            total: self.total_frames,
        }
    }

    /// Share of frames dropped by the network, since the stream started.
    pub fn drop_percent(&self) -> f64 {
        self.dropped().percent()
    }

    /// Encoder backpressure as a percentage rather than the `0.0..=1.0`
    /// fraction OBS reports.
    pub fn congestion_percent(&self) -> f64 {
        self.congestion * 100.0
    }
}

/// `GetRecordStatus` — local recording output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordStatus {
    pub active: bool,
    pub paused: bool,
    pub timecode: String,
    pub duration_ms: u64,
    pub bytes: u64,
}

impl RecordStatus {
    pub fn from_value(value: &Value) -> Self {
        Self {
            active: flag(value, "outputActive"),
            paused: flag(value, "outputPaused"),
            timecode: text(value, "outputTimecode"),
            duration_ms: count(value, "outputDuration"),
            bytes: count(value, "outputBytes"),
        }
    }
}

/// `GetVideoSettings` — canvas and output geometry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoSettings {
    pub base_width: u32,
    pub base_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub fps_numerator: u32,
    pub fps_denominator: u32,
}

impl VideoSettings {
    pub fn from_value(value: &Value) -> Self {
        Self {
            base_width: count(value, "baseWidth") as u32,
            base_height: count(value, "baseHeight") as u32,
            output_width: count(value, "outputWidth") as u32,
            output_height: count(value, "outputHeight") as u32,
            fps_numerator: count(value, "fpsNumerator") as u32,
            fps_denominator: count(value, "fpsDenominator") as u32,
        }
    }

    /// The FPS OBS is *configured* for — the target `activeFps` is judged
    /// against. Zero when OBS has not reported video settings yet.
    pub fn target_fps(&self) -> f64 {
        if self.fps_denominator == 0 {
            0.0
        } else {
            f64::from(self.fps_numerator) / f64::from(self.fps_denominator)
        }
    }

    /// `1920x1080 → 1280x720` style summary of the scaling in effect.
    ///
    /// The arrow is supplied by the caller so the UI's ASCII mode can pass
    /// `->` instead; this layer has no opinion about glyphs.
    pub fn geometry(&self, arrow: &str) -> String {
        format!(
            "{}x{} {arrow} {}x{}",
            self.base_width, self.base_height, self.output_width, self.output_height
        )
    }

    /// Whether OBS is downscaling the canvas for output.
    pub fn is_downscaled(&self) -> bool {
        self.output_width < self.base_width || self.output_height < self.base_height
    }
}

/// `GetVersion` — identifies the OBS instance in the header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Version {
    pub obs_version: String,
    pub websocket_version: String,
    pub platform: String,
}

impl Version {
    pub fn from_value(value: &Value) -> Self {
        Self {
            obs_version: text(value, "obsVersion"),
            websocket_version: text(value, "obsWebSocketVersion"),
            platform: text(value, "platform"),
        }
    }
}

/// One entry from `GetSceneList`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub name: String,
    pub index: usize,
}

/// `GetSceneList` — program/preview scene plus the full list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneList {
    pub current_program: String,
    pub current_preview: String,
    pub scenes: Vec<Scene>,
}

impl SceneList {
    pub fn from_value(value: &Value) -> Self {
        // OBS lists scenes bottom-up, matching the scene dock's own ordering;
        // reverse so the dashboard reads top-down like the OBS UI.
        let mut scenes: Vec<Scene> = value
            .get("scenes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| Scene {
                        name: text(item, "sceneName"),
                        index: count(item, "sceneIndex") as usize,
                    })
                    .collect()
            })
            .unwrap_or_default();
        scenes.sort_by_key(|scene| std::cmp::Reverse(scene.index));

        Self {
            current_program: text(value, "currentProgramSceneName"),
            current_preview: text(value, "currentPreviewSceneName"),
            scenes,
        }
    }
}

/// An audio-capable input, merged from `GetInputList` + volume/mute lookups
/// and refreshed in place by `InputVolumeMeters` events.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioInput {
    pub name: String,
    pub kind: String,
    pub muted: bool,
    /// Fader position as a multiplier (`1.0` = 0 dB).
    pub volume_mul: f64,
    /// Fader position in dB, as OBS reports it (`-inf` becomes `-100.0`).
    pub volume_db: f64,
    /// Most recent peak level per channel, linear `0.0..=1.0`.
    pub peaks: Vec<f32>,
}

impl AudioInput {
    /// Loudest channel of the last meter frame, linear.
    pub fn peak(&self) -> f32 {
        self.peaks.iter().copied().fold(0.0_f32, f32::max)
    }
}

/// Whether an input kind carries audio worth metering.
///
/// OBS reports image sources, browser docks and colour sources in
/// `GetInputList` too; metering them would fill the panel with permanent
/// silence, so the audio view filters on this.
pub fn is_audio_kind(kind: &str) -> bool {
    const AUDIO_KINDS: [&str; 8] = [
        "wasapi_input_capture",
        "wasapi_output_capture",
        "wasapi_process_output_capture",
        "pulse_input_capture",
        "pulse_output_capture",
        "coreaudio_input_capture",
        "coreaudio_output_capture",
        "alsa_input_capture",
    ];
    // Media and browser sources are audio-bearing but not named consistently
    // across platforms, so match them by substring as well.
    AUDIO_KINDS.contains(&kind)
        || kind.contains("audio")
        || kind.contains("ffmpeg_source")
        || kind.contains("browser_source")
        || kind.contains("vlc_source")
}

fn number(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}

fn count(value: &Value, key: &str) -> u64 {
    // OBS sends these as JSON numbers that may be typed as floats; accept both
    // and never let a negative or non-finite value wrap around on cast.
    value
        .get(key)
        .and_then(|v| {
            v.as_u64().or_else(|| {
                v.as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .map(|n| n as u64)
            })
        })
        .unwrap_or(0)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stats_decode_a_full_payload() {
        let stats = Stats::from_value(&json!({
            "cpuUsage": 4.2,
            "memoryUsage": 1536.5,
            "availableDiskSpace": 250_000.0,
            "activeFps": 59.94,
            "averageFrameRenderTime": 2.31,
            "renderSkippedFrames": 12,
            "renderTotalFrames": 120_000,
            "outputSkippedFrames": 3,
            "outputTotalFrames": 60_000,
            "webSocketSessionIncomingMessages": 42,
            "webSocketSessionOutgoingMessages": 43
        }));
        assert_eq!(stats.cpu_usage, 4.2);
        assert_eq!(stats.active_fps, 59.94);
        assert_eq!(stats.render_total_frames, 120_000);
        assert_eq!(stats.websocket_outgoing_messages, 43);
    }

    #[test]
    fn stats_default_every_missing_field() {
        let stats = Stats::from_value(&json!({}));
        assert_eq!(stats, Stats::default());
        assert_eq!(stats.render_skip_percent(), 0.0);
        assert_eq!(stats.output_skip_percent(), 0.0);
    }

    #[test]
    fn stats_ignore_non_finite_and_negative_numbers() {
        // serde_json cannot represent NaN, but a wrong-typed field can appear.
        let stats = Stats::from_value(&json!({
            "cpuUsage": "not a number",
            "renderTotalFrames": -5.0
        }));
        assert_eq!(stats.cpu_usage, 0.0);
        assert_eq!(stats.render_total_frames, 0);
    }

    #[test]
    fn skip_percentages_are_relative_to_totals() {
        let stats = Stats {
            render_skipped_frames: 5,
            render_total_frames: 100,
            output_skipped_frames: 1,
            output_total_frames: 50,
            ..Stats::default()
        };
        assert_eq!(stats.render_skip_percent(), 5.0);
        assert_eq!(stats.output_skip_percent(), 2.0);
    }

    #[test]
    fn stream_status_decodes_and_derives_drop_rate() {
        let status = StreamStatus::from_value(&json!({
            "outputActive": true,
            "outputReconnecting": false,
            "outputTimecode": "00:12:34.567",
            "outputDuration": 754_567,
            "outputCongestion": 0.25,
            "outputBytes": 900_000_000u64,
            "outputSkippedFrames": 20,
            "outputTotalFrames": 1000
        }));
        assert!(status.active);
        assert_eq!(status.duration_ms, 754_567);
        assert_eq!(status.drop_percent(), 2.0);
        assert_eq!(status.congestion, 0.25);
    }

    #[test]
    fn inactive_stream_status_is_all_zeroes() {
        let status = StreamStatus::from_value(&json!({"outputActive": false}));
        assert!(!status.active);
        assert_eq!(status.bytes, 0);
        assert_eq!(status.drop_percent(), 0.0);
    }

    #[test]
    fn record_status_tracks_pause() {
        let status = RecordStatus::from_value(&json!({
            "outputActive": true, "outputPaused": true,
            "outputDuration": 1000, "outputBytes": 2048
        }));
        assert!(status.active && status.paused);
        assert_eq!(status.bytes, 2048);
    }

    #[test]
    fn video_settings_compute_target_fps_and_geometry() {
        let settings = VideoSettings::from_value(&json!({
            "baseWidth": 1920, "baseHeight": 1080,
            "outputWidth": 1280, "outputHeight": 720,
            "fpsNumerator": 60, "fpsDenominator": 1
        }));
        assert_eq!(settings.target_fps(), 60.0);
        assert_eq!(settings.geometry("→"), "1920x1080 → 1280x720");
        assert_eq!(settings.geometry("->"), "1920x1080 -> 1280x720");
        assert!(settings.is_downscaled());
    }

    #[test]
    fn video_settings_survive_a_zero_denominator() {
        let settings = VideoSettings::from_value(&json!({"fpsNumerator": 60, "fpsDenominator": 0}));
        assert_eq!(settings.target_fps(), 0.0);
    }

    #[test]
    fn ntsc_frame_rates_keep_their_fraction() {
        let settings = VideoSettings::from_value(&json!({
            "fpsNumerator": 60_000, "fpsDenominator": 1001
        }));
        assert!((settings.target_fps() - 59.94).abs() < 0.01);
    }

    #[test]
    fn scene_list_is_reordered_top_down() {
        let list = SceneList::from_value(&json!({
            "currentProgramSceneName": "Live",
            "currentPreviewSceneName": "Intro",
            "scenes": [
                {"sceneName": "Outro", "sceneIndex": 0},
                {"sceneName": "Live",  "sceneIndex": 1},
                {"sceneName": "Intro", "sceneIndex": 2}
            ]
        }));
        assert_eq!(list.current_program, "Live");
        let names: Vec<&str> = list.scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Intro", "Live", "Outro"]);
    }

    #[test]
    fn scene_list_tolerates_a_missing_scenes_array() {
        let list = SceneList::from_value(&json!({"currentProgramSceneName": "Solo"}));
        assert_eq!(list.current_program, "Solo");
        assert!(list.scenes.is_empty());
    }

    #[test]
    fn audio_kinds_exclude_silent_sources() {
        assert!(is_audio_kind("pulse_input_capture"));
        assert!(is_audio_kind("wasapi_output_capture"));
        assert!(is_audio_kind("coreaudio_input_capture"));
        assert!(is_audio_kind("ffmpeg_source"));
        assert!(is_audio_kind("browser_source"));
        assert!(!is_audio_kind("image_source"));
        assert!(!is_audio_kind("color_source_v3"));
        assert!(!is_audio_kind("text_ft2_source_v2"));
    }

    #[test]
    fn audio_input_peak_is_the_loudest_channel() {
        let input = AudioInput {
            peaks: vec![0.1, 0.7, 0.3],
            ..AudioInput::default()
        };
        assert!((input.peak() - 0.7).abs() < f32::EPSILON);
        assert_eq!(AudioInput::default().peak(), 0.0);
    }
}

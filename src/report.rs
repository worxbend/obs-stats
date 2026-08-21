//! The JSON report printed by `obs-stats once`.
//!
//! This module exists so the shape of that output has one owner. `once` is
//! meant to be piped into `jq` or a monitoring script, which makes its key
//! names a public contract the same way the exit codes are — renaming
//! `disk_free_mib` breaks someone's dashboard just as surely as changing an
//! exit code breaks their shell script.
//!
//! It lives in the library rather than in `main.rs` for two reasons. A binary
//! crate cannot be imported, so a contract defined there can only be tested
//! from inside itself and can never be reused. And keeping it beside the
//! `Snapshot` it serialises means the two are read together when either
//! changes.
//!
//! Units are named in the keys — `_mib`, `_ms`, `_percent` — because a bare
//! number in JSON carries none.

use crate::metrics::Snapshot;
use crate::util;

/// The documented JSON shape of `obs-stats once`.
pub fn as_json(snapshot: &Snapshot) -> serde_json::Value {
    let stats = &snapshot.stats;
    serde_json::json!({
        "obs": {
            "version": snapshot.version.obs_version,
            "websocket_version": snapshot.version.websocket_version,
            "platform": snapshot.version.platform,
        },
        "system": {
            "cpu_percent": stats.cpu_usage,
            "memory_mib": stats.memory_usage,
            "disk_free_mib": stats.available_disk_space,
        },
        "video": {
            "active_fps": stats.active_fps,
            "target_fps": snapshot.video.target_fps(),
            "average_frame_render_ms": stats.average_frame_render_time,
            "frame_budget_used": snapshot.frame_budget_ratio(),
            "base": format!("{}x{}", snapshot.video.base_width, snapshot.video.base_height),
            "output": format!("{}x{}", snapshot.video.output_width, snapshot.video.output_height),
        },
        "frames": {
            "render_skipped": stats.render_skipped_frames,
            "render_total": stats.render_total_frames,
            "output_skipped": stats.output_skipped_frames,
            "output_total": stats.output_total_frames,
        },
        "stream": {
            "active": snapshot.stream.active,
            "reconnecting": snapshot.stream.reconnecting,
            "duration_ms": snapshot.stream.duration_ms,
            "duration": util::duration_ms(snapshot.stream.duration_ms),
            "bytes": snapshot.stream.bytes,
            "congestion": snapshot.stream.congestion,
            "dropped_frames": snapshot.stream.skipped_frames,
            "total_frames": snapshot.stream.total_frames,
            "drop_percent": snapshot.stream.drop_percent(),
        },
        "record": {
            "active": snapshot.record.active,
            "paused": snapshot.record.paused,
            "duration_ms": snapshot.record.duration_ms,
            "duration": util::duration_ms(snapshot.record.duration_ms),
            "bytes": snapshot.record.bytes,
        },
        "scenes": {
            "program": snapshot.scenes.current_program,
            "preview": snapshot.scenes.current_preview,
            "all": snapshot.scenes.scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        },
        "profile": snapshot.current_profile,
        "collection": snapshot.current_collection,
        "inputs": snapshot.inputs.iter().map(|input| serde_json::json!({
            "name": input.name,
            "kind": input.kind,
            "muted": input.muted,
            "volume_db": if input.volume_db.is_finite() { Some(input.volume_db) } else { None },
        })).collect::<Vec<_>>(),
        "health": snapshot.health().label(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::stats::{AudioInput, SceneList, Stats, StreamStatus, VideoSettings};

    fn populated() -> Snapshot {
        let mut snapshot = Snapshot::new(8);
        snapshot.stats = Stats {
            cpu_usage: 12.5,
            memory_usage: 1500.0,
            available_disk_space: 250_000.0,
            active_fps: 59.94,
            average_frame_render_time: 3.1,
            render_skipped_frames: 4,
            render_total_frames: 10_000,
            ..Stats::default()
        };
        snapshot.video = VideoSettings {
            base_width: 1920,
            base_height: 1080,
            output_width: 1280,
            output_height: 720,
            fps_numerator: 60,
            fps_denominator: 1,
        };
        snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 65_000,
            bytes: 1000,
            skipped_frames: 2,
            total_frames: 200,
            ..StreamStatus::default()
        };
        snapshot.scenes = SceneList {
            current_program: "Live".into(),
            ..SceneList::default()
        };
        snapshot.current_profile = "Twitch".into();
        snapshot.inputs = vec![AudioInput {
            name: "Mic".into(),
            kind: "pulse_input_capture".into(),
            muted: false,
            volume_db: -6.0,
            ..AudioInput::default()
        }];
        snapshot
    }

    #[test]
    fn json_output_covers_every_documented_section() {
        let json = as_json(&populated());
        for key in [
            "obs",
            "system",
            "video",
            "frames",
            "stream",
            "record",
            "scenes",
            "profile",
            "collection",
            "inputs",
            "health",
        ] {
            assert!(json.get(key).is_some(), "missing section {key}");
        }
    }

    #[test]
    fn json_output_carries_the_real_numbers() {
        let json = as_json(&populated());
        assert_eq!(json["system"]["cpu_percent"], 12.5);
        assert_eq!(json["video"]["active_fps"], 59.94);
        assert_eq!(json["video"]["target_fps"], 60.0);
        assert_eq!(json["video"]["base"], "1920x1080");
        assert_eq!(json["video"]["output"], "1280x720");
        assert_eq!(json["frames"]["render_skipped"], 4);
        assert_eq!(json["stream"]["active"], true);
        assert_eq!(json["stream"]["duration"], "01:05");
        assert_eq!(json["stream"]["drop_percent"], 1.0);
        assert_eq!(json["scenes"]["program"], "Live");
        assert_eq!(json["profile"], "Twitch");
    }

    #[test]
    fn json_output_lists_inputs_with_their_fader_positions() {
        let json = as_json(&populated());
        let inputs = json["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["name"], "Mic");
        assert_eq!(inputs[0]["muted"], false);
        assert_eq!(inputs[0]["volume_db"], -6.0);
    }

    #[test]
    fn an_unknown_fader_position_serialises_as_null_not_nan() {
        // serde_json cannot represent -inf, so it must be normalised away.
        let mut snapshot = populated();
        snapshot.inputs[0].volume_db = f64::NEG_INFINITY;
        let json = as_json(&snapshot);
        assert!(json["inputs"][0]["volume_db"].is_null());
        // The whole document must still serialise.
        serde_json::to_string(&json).expect("must serialise");
    }

    #[test]
    fn an_empty_snapshot_still_produces_valid_json() {
        let json = as_json(&Snapshot::new(8));
        let text = serde_json::to_string_pretty(&json).unwrap();
        assert!(text.contains("\"health\""));
        assert_eq!(json["stream"]["active"], false);
        assert_eq!(json["inputs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_health_verdict_is_reported_as_a_word() {
        let json = as_json(&populated());
        // 2 of 200 frames dropped is 1%, which is the "dropping" threshold.
        assert_eq!(json["health"], "DROPPING");
        assert_eq!(as_json(&Snapshot::new(8))["health"], "HEALTHY");
    }
}

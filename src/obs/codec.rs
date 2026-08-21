//! Pure request building and response decoding for the obs-websocket wire.
//!
//! This module knows the shape of every request `obs-stats` sends and every
//! response it reads back, and nothing else. There is no socket here, no
//! `async`, and no channel — give it a `serde_json::Value` and it hands back
//! a typed payload, which is what makes the whole wire format testable
//! without a live OBS.
//!
//! The counterpart is [`super::client`], which owns the socket and drives the
//! session. Keeping the two apart means a change to *what* obs-websocket
//! sends never has to be made inside reconnect-and-backoff logic, and vice
//! versa.
//!
//! # Decoders are total
//!
//! Nothing in this module returns an error for a field that is missing, has
//! the wrong type, or belongs to a request that failed. Decoding falls back
//! to a default instead. OBS adds fields between point releases and drops
//! others when an output is inactive; a dashboard that blanks out because one
//! number moved is worse than one showing a zero.

use serde_json::{Value, json};

use crate::obs::client::{Inventory, LogLevel, PollData};
use crate::obs::protocol::{self, Envelope, Request, RequestBatch, RequestResponse};
use crate::obs::stats::{
    AudioInput, RecordStatus, SceneList, Stats, StreamStatus, VideoSettings, is_audio_kind,
};

pub(crate) const REQUEST_ID_POLL: &str = "obs-stats-poll";
pub(crate) const REQUEST_ID_INVENTORY: &str = "obs-stats-inventory";
pub(crate) const REQUEST_ID_AUDIO: &str = "obs-stats-audio";
pub(crate) const REQUEST_ID_VERSION: &str = "obs-stats-version";

/// Turn an OBS event into a log line, or `None` for events too noisy to show.
///
/// Kept free of I/O so the mapping can be unit-tested directly.
pub(crate) fn describe_event(event_type: &str, data: &Value) -> Option<(LogLevel, String)> {
    let name = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };

    let described = match event_type {
        "CurrentProgramSceneChanged" => (LogLevel::Info, format!("scene → {}", name("sceneName"))),
        "CurrentPreviewSceneChanged" => {
            (LogLevel::Info, format!("preview → {}", name("sceneName")))
        }
        "CurrentProfileChanged" => (LogLevel::Info, format!("profile → {}", name("profileName"))),
        "CurrentSceneCollectionChanged" => (
            LogLevel::Info,
            format!("collection → {}", name("sceneCollectionName")),
        ),
        "StreamStateChanged" => {
            let state = name("outputState");
            let level = if state.contains("RECONNECT") {
                LogLevel::Warn
            } else {
                LogLevel::Info
            };
            (level, format!("stream {}", humanize_output_state(&state)))
        }
        "RecordStateChanged" => (
            LogLevel::Info,
            format!("record {}", humanize_output_state(&name("outputState"))),
        ),
        "InputMuteStateChanged" => {
            let muted = data
                .get("inputMuted")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (
                LogLevel::Info,
                format!(
                    "{} {}",
                    name("inputName"),
                    if muted { "muted" } else { "unmuted" }
                ),
            )
        }
        "InputCreated" => (
            LogLevel::Info,
            format!("input added: {}", name("inputName")),
        ),
        "InputRemoved" => (
            LogLevel::Warn,
            format!("input removed: {}", name("inputName")),
        ),
        "SceneCreated" => (
            LogLevel::Info,
            format!("scene added: {}", name("sceneName")),
        ),
        "SceneRemoved" => (
            LogLevel::Warn,
            format!("scene removed: {}", name("sceneName")),
        ),
        "ExitStarted" => (LogLevel::Warn, "OBS is shutting down".to_string()),
        _ => return None,
    };

    Some(described)
}

/// `OBS_WEBSOCKET_OUTPUT_STARTED` → `started`.
fn humanize_output_state(state: &str) -> String {
    state
        .strip_prefix("OBS_WEBSOCKET_OUTPUT_")
        .unwrap_or(state)
        .to_ascii_lowercase()
}
fn batch(request_id: &str, requests: Vec<Request>) -> Envelope {
    Envelope {
        op: protocol::OP_REQUEST_BATCH,
        d: serde_json::to_value(RequestBatch {
            request_id: request_id.to_string(),
            // One failing member (a stopped output, say) must not cancel the
            // rest of the batch.
            halt_on_failure: false,
            requests,
        })
        .unwrap_or(Value::Null),
    }
}

fn request(request_type: &str, request_id: &str, data: Option<Value>) -> Request {
    Request {
        request_type: request_type.to_string(),
        request_id: request_id.to_string(),
        request_data: data,
    }
}

/// The per-tick batch: counters plus both output states.
pub(crate) fn hot_poll_batch() -> Envelope {
    batch(
        REQUEST_ID_POLL,
        vec![
            request("GetStats", "stats", None),
            request("GetStreamStatus", "stream", None),
            request("GetRecordStatus", "record", None),
        ],
    )
}

/// The slow batch: everything that changes on human timescales.
pub(crate) fn inventory_batch() -> Envelope {
    batch(
        REQUEST_ID_INVENTORY,
        vec![
            request("GetVideoSettings", "video", None),
            request("GetSceneList", "scenes", None),
            request("GetInputList", "inputs", None),
            request("GetProfileList", "profiles", None),
            request("GetSceneCollectionList", "collections", None),
        ],
    )
}

/// Volume and mute lookups for each audio input, as one batch.
///
/// The request id of each member is `mute:<name>` / `vol:<name>` so the
/// response can be attributed without relying on ordering.
pub(crate) fn audio_batch(inputs: &[AudioInput]) -> Envelope {
    let mut requests = Vec::with_capacity(inputs.len() * 2);
    for input in inputs {
        let data = Some(json!({"inputName": input.name}));
        requests.push(request(
            "GetInputMute",
            &format!("mute:{}", input.name),
            data.clone(),
        ));
        requests.push(request(
            "GetInputVolume",
            &format!("vol:{}", input.name),
            data,
        ));
    }
    batch(REQUEST_ID_AUDIO, requests)
}

/// Decode a hot-poll batch response by request type.
pub(crate) fn decode_poll(results: &[RequestResponse]) -> PollData {
    let mut poll = PollData::default();
    for result in results {
        let Some(data) = result.ok_data() else {
            continue;
        };
        match result.request_type.as_str() {
            "GetStats" => poll.stats = Stats::from_value(data),
            "GetStreamStatus" => poll.stream = StreamStatus::from_value(data),
            "GetRecordStatus" => poll.record = RecordStatus::from_value(data),
            _ => {}
        }
    }
    poll
}

/// Decode an inventory batch response.
pub(crate) fn decode_inventory(results: &[RequestResponse]) -> Inventory {
    let mut inventory = Inventory::default();
    for result in results {
        let Some(data) = result.ok_data() else {
            continue;
        };
        match result.request_type.as_str() {
            "GetVideoSettings" => inventory.video = VideoSettings::from_value(data),
            "GetSceneList" => inventory.scenes = SceneList::from_value(data),
            "GetInputList" => inventory.inputs = decode_input_list(data),
            "GetProfileList" => {
                inventory.current_profile = string_field(data, "currentProfileName");
                inventory.profiles = string_list(data, "profiles");
            }
            "GetSceneCollectionList" => {
                inventory.current_collection = string_field(data, "currentSceneCollectionName");
                inventory.collections = string_list(data, "sceneCollections");
            }
            _ => {}
        }
    }
    inventory
}

fn decode_input_list(data: &Value) -> Vec<AudioInput> {
    data.get("inputs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = item
                        .get("inputKind")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !is_audio_kind(kind) {
                        return None;
                    }
                    Some(AudioInput {
                        name: item
                            .get("inputName")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        kind: kind.to_string(),
                        volume_db: f64::NEG_INFINITY,
                        ..AudioInput::default()
                    })
                })
                .filter(|input| !input.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Extract per-input peak levels from an `InputVolumeMeters` event.
///
/// OBS sends `inputLevelsMul` as one array per channel, each holding
/// `[magnitude, peak, input_peak]`; the dashboard shows the peak.
pub(crate) fn decode_meter_event(data: &Value) -> Option<Vec<(String, Vec<f32>)>> {
    let inputs = data.get("inputs")?.as_array()?;
    let levels = inputs
        .iter()
        .filter_map(|input| {
            let name = input.get("inputName")?.as_str()?.to_string();
            let channels = input
                .get("inputLevelsMul")?
                .as_array()?
                .iter()
                .map(|channel| {
                    channel
                        .as_array()
                        .and_then(|values| values.get(1))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0) as f32
                })
                .collect();
            Some((name, channels))
        })
        .collect();
    Some(levels)
}

/// Decode the mute/volume follow-up batch into `(name, [muted, volume_db])`
/// tuples, encoded in the same `Meters` shape the UI merges.
///
/// Mute is carried as `f32::NAN` in slot 0 when unknown; the app layer reads
/// these through [`crate::metrics::Snapshot::apply_audio_probe`].
pub(crate) fn decode_audio_levels(results: &[RequestResponse]) -> Option<Vec<(String, Vec<f32>)>> {
    let mut probes: Vec<(String, Vec<f32>)> = Vec::new();
    for result in results {
        let Some(data) = result.ok_data() else {
            continue;
        };
        let Some((tag, name)) = result.request_id.split_once(':') else {
            continue;
        };
        let entry = match probes.iter_mut().find(|(existing, _)| existing == name) {
            Some(entry) => entry,
            None => {
                probes.push((name.to_string(), vec![f32::NAN, f32::NAN]));
                probes.last_mut()?
            }
        };
        match tag {
            "mute" => {
                let muted = data.get("inputMuted").and_then(Value::as_bool);
                entry.1[0] = match muted {
                    Some(true) => 1.0,
                    Some(false) => 0.0,
                    None => f32::NAN,
                };
            }
            "vol" => {
                entry.1[1] = data
                    .get("inputVolumeDb")
                    .and_then(Value::as_f64)
                    .map(|db| db as f32)
                    .unwrap_or(f32::NAN);
            }
            _ => {}
        }
    }

    if probes.is_empty() {
        None
    } else {
        Some(probes)
    }
}

fn string_field(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn string_list(data: &Value, key: &str) -> Vec<String> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    // OBS 5.x returns plain strings here, but older builds
                    // wrapped them in objects; accept both.
                    item.as_str().map(str::to_string).or_else(|| {
                        item.get("profileName")
                            .or_else(|| item.get("sceneCollectionName"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(request_type: &str, request_id: &str, data: Value) -> RequestResponse {
        serde_json::from_value(json!({
            "requestType": request_type,
            "requestId": request_id,
            "requestStatus": {"result": true, "code": 100},
            "responseData": data
        }))
        .unwrap()
    }

    fn failed(request_type: &str) -> RequestResponse {
        serde_json::from_value(json!({
            "requestType": request_type,
            "requestId": "x",
            "requestStatus": {"result": false, "code": 604, "comment": "no output"}
        }))
        .unwrap()
    }

    #[test]
    fn hot_poll_batch_asks_for_the_three_hot_endpoints() {
        let envelope = hot_poll_batch();
        assert_eq!(envelope.op, protocol::OP_REQUEST_BATCH);
        assert_eq!(envelope.d["requestId"], REQUEST_ID_POLL);
        assert_eq!(envelope.d["haltOnFailure"], false);
        let types: Vec<&str> = envelope.d["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["requestType"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["GetStats", "GetStreamStatus", "GetRecordStatus"]);
    }

    #[test]
    fn inventory_batch_covers_scenes_inputs_and_settings() {
        let envelope = inventory_batch();
        let types: Vec<&str> = envelope.d["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["requestType"].as_str().unwrap())
            .collect();
        assert!(types.contains(&"GetVideoSettings"));
        assert!(types.contains(&"GetSceneList"));
        assert!(types.contains(&"GetInputList"));
        assert!(types.contains(&"GetProfileList"));
        assert!(types.contains(&"GetSceneCollectionList"));
    }

    #[test]
    fn audio_batch_pairs_mute_and_volume_per_input() {
        let inputs = vec![
            AudioInput {
                name: "Mic".into(),
                ..AudioInput::default()
            },
            AudioInput {
                name: "Desktop".into(),
                ..AudioInput::default()
            },
        ];
        let envelope = audio_batch(&inputs);
        let requests = envelope.d["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0]["requestId"], "mute:Mic");
        assert_eq!(requests[1]["requestId"], "vol:Mic");
        assert_eq!(requests[1]["requestData"]["inputName"], "Mic");
        assert_eq!(requests[3]["requestId"], "vol:Desktop");
    }

    #[test]
    fn audio_batch_is_empty_for_no_inputs() {
        let envelope = audio_batch(&[]);
        assert!(envelope.d["requests"].as_array().unwrap().is_empty());
    }

    #[test]
    fn decode_poll_maps_each_member_by_request_type() {
        let poll = decode_poll(&[
            response(
                "GetStats",
                "stats",
                json!({"cpuUsage": 7.5, "activeFps": 60.0}),
            ),
            response("GetStreamStatus", "stream", json!({"outputActive": true})),
            response("GetRecordStatus", "record", json!({"outputActive": false})),
        ]);
        assert_eq!(poll.stats.cpu_usage, 7.5);
        assert!(poll.stream.active);
        assert!(!poll.record.active);
    }

    #[test]
    fn decode_poll_skips_failed_members() {
        let poll = decode_poll(&[
            response("GetStats", "stats", json!({"cpuUsage": 7.5})),
            failed("GetStreamStatus"),
        ]);
        assert_eq!(poll.stats.cpu_usage, 7.5);
        assert_eq!(poll.stream, StreamStatus::default());
    }

    #[test]
    fn decode_inventory_filters_inputs_to_audio_kinds() {
        let inventory = decode_inventory(&[
            response(
                "GetInputList",
                "inputs",
                json!({"inputs": [
                    {"inputName": "Mic", "inputKind": "pulse_input_capture"},
                    {"inputName": "Logo", "inputKind": "image_source"},
                    {"inputName": "Music", "inputKind": "ffmpeg_source"}
                ]}),
            ),
            response(
                "GetProfileList",
                "profiles",
                json!({"currentProfileName": "Twitch", "profiles": ["Twitch", "YouTube"]}),
            ),
        ]);
        let names: Vec<&str> = inventory.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Mic", "Music"]);
        assert_eq!(inventory.current_profile, "Twitch");
        assert_eq!(inventory.profiles, ["Twitch", "YouTube"]);
    }

    #[test]
    fn string_list_accepts_object_wrapped_legacy_entries() {
        let legacy = json!({"profiles": [{"profileName": "Old"}, "New"]});
        assert_eq!(string_list(&legacy, "profiles"), ["Old", "New"]);
    }

    #[test]
    fn meter_event_extracts_the_peak_channel_slot() {
        let levels = decode_meter_event(&json!({
            "inputs": [{
                "inputName": "Mic",
                "inputLevelsMul": [[0.1, 0.5, 0.6], [0.2, 0.4, 0.5]]
            }]
        }))
        .unwrap();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].0, "Mic");
        assert_eq!(levels[0].1, vec![0.5, 0.4]);
    }

    #[test]
    fn meter_event_without_inputs_is_none() {
        assert!(decode_meter_event(&json!({})).is_none());
    }

    #[test]
    fn muted_input_reports_empty_levels_not_an_error() {
        // OBS sends an empty level array for a muted source.
        let levels =
            decode_meter_event(&json!({"inputs": [{"inputName": "Mic", "inputLevelsMul": []}]}))
                .unwrap();
        assert_eq!(levels[0].1, Vec::<f32>::new());
    }

    #[test]
    fn audio_probe_merges_mute_and_volume_per_input() {
        let probes = decode_audio_levels(&[
            response("GetInputMute", "mute:Mic", json!({"inputMuted": true})),
            response("GetInputVolume", "vol:Mic", json!({"inputVolumeDb": -6.5})),
        ])
        .unwrap();
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].0, "Mic");
        assert_eq!(probes[0].1[0], 1.0);
        assert!((probes[0].1[1] + 6.5).abs() < 1e-5);
    }

    #[test]
    fn audio_probe_handles_names_containing_a_colon() {
        // split_once takes the first colon, so the tag stays intact and the
        // remainder — colons and all — is the input name.
        let probes = decode_audio_levels(&[response(
            "GetInputMute",
            "mute:Mic: Main",
            json!({"inputMuted": false}),
        )])
        .unwrap();
        assert_eq!(probes[0].0, "Mic: Main");
        assert_eq!(probes[0].1[0], 0.0);
    }

    #[test]
    fn audio_probe_is_none_when_nothing_decoded() {
        assert!(decode_audio_levels(&[]).is_none());
        assert!(decode_audio_levels(&[failed("GetInputMute")]).is_none());
    }

    #[test]
    fn output_states_are_humanized() {
        assert_eq!(
            humanize_output_state("OBS_WEBSOCKET_OUTPUT_STARTED"),
            "started"
        );
        assert_eq!(
            humanize_output_state("OBS_WEBSOCKET_OUTPUT_RECONNECTING"),
            "reconnecting"
        );
        assert_eq!(humanize_output_state("weird"), "weird");
    }

    #[test]
    fn scene_change_events_become_info_log_lines() {
        let (level, message) =
            describe_event("CurrentProgramSceneChanged", &json!({"sceneName": "Live"})).unwrap();
        assert_eq!(level, LogLevel::Info);
        assert_eq!(message, "scene → Live");
    }

    #[test]
    fn reconnecting_stream_events_are_warnings() {
        let (level, message) = describe_event(
            "StreamStateChanged",
            &json!({"outputState": "OBS_WEBSOCKET_OUTPUT_RECONNECTING"}),
        )
        .unwrap();
        assert_eq!(level, LogLevel::Warn);
        assert_eq!(message, "stream reconnecting");
    }

    #[test]
    fn removals_are_warnings_and_mute_reads_naturally() {
        let (level, message) =
            describe_event("InputRemoved", &json!({"inputName": "Mic"})).unwrap();
        assert_eq!(level, LogLevel::Warn);
        assert_eq!(message, "input removed: Mic");

        let (_, muted) = describe_event(
            "InputMuteStateChanged",
            &json!({"inputName": "Mic", "inputMuted": true}),
        )
        .unwrap();
        assert_eq!(muted, "Mic muted");
    }

    #[test]
    fn uninteresting_events_are_dropped() {
        assert!(describe_event("SceneItemTransformChanged", &json!({})).is_none());
        assert!(describe_event("InputVolumeMeters", &json!({})).is_none());
    }

    #[test]
    fn events_with_missing_names_still_render() {
        let (_, message) = describe_event("CurrentProgramSceneChanged", &json!({})).unwrap();
        assert_eq!(message, "scene → ?");
    }
}

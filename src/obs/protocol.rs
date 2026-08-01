//! Wire types for the obs-websocket 5.x protocol.
//!
//! Only the opcodes `obs-stats` actually exchanges are modelled. Everything
//! that varies per request type stays as a `serde_json::Value` and is decoded
//! in [`crate::obs::stats`], so a new OBS field never breaks the transport.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const OP_HELLO: u8 = 0;
pub const OP_IDENTIFY: u8 = 1;
pub const OP_IDENTIFIED: u8 = 2;
pub const OP_EVENT: u8 = 5;
pub const OP_REQUEST: u8 = 6;
pub const OP_REQUEST_RESPONSE: u8 = 7;
pub const OP_REQUEST_BATCH: u8 = 8;
pub const OP_REQUEST_BATCH_RESPONSE: u8 = 9;

/// RPC version this client negotiates. obs-websocket 5.x speaks version 1.
pub const RPC_VERSION: u32 = 1;

/// `EventSubscription` bit flags.
///
/// `ALL` covers the low-volume categories; the high-volume ones (volume
/// meters, transform changes) are opt-in bits above them, and we take only
/// `INPUT_VOLUME_METERS` since that is what drives the audio panel.
pub mod events {
    pub const GENERAL: u32 = 1 << 0;
    pub const CONFIG: u32 = 1 << 1;
    pub const SCENES: u32 = 1 << 2;
    pub const INPUTS: u32 = 1 << 3;
    pub const TRANSITIONS: u32 = 1 << 4;
    pub const FILTERS: u32 = 1 << 5;
    pub const OUTPUTS: u32 = 1 << 6;
    pub const SCENE_ITEMS: u32 = 1 << 7;
    pub const MEDIA_INPUTS: u32 = 1 << 8;
    pub const VENDORS: u32 = 1 << 9;
    pub const UI: u32 = 1 << 10;
    pub const ALL: u32 = GENERAL
        | CONFIG
        | SCENES
        | INPUTS
        | TRANSITIONS
        | FILTERS
        | OUTPUTS
        | SCENE_ITEMS
        | MEDIA_INPUTS
        | VENDORS
        | UI;

    pub const INPUT_VOLUME_METERS: u32 = 1 << 16;

    /// What `obs-stats` subscribes to: every low-volume event, plus volume
    /// meters for the audio panel.
    pub const DASHBOARD: u32 = ALL | INPUT_VOLUME_METERS;
}

/// The outer envelope every frame shares: an opcode and a payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub op: u8,
    pub d: Value,
}

#[derive(Debug, Deserialize)]
pub struct Hello {
    #[serde(rename = "obsWebSocketVersion")]
    pub obs_websocket_version: String,
    #[serde(rename = "rpcVersion")]
    pub rpc_version: u32,
    #[serde(default)]
    pub authentication: Option<HelloAuthentication>,
}

#[derive(Debug, Deserialize)]
pub struct HelloAuthentication {
    pub challenge: String,
    pub salt: String,
}

#[derive(Debug, Serialize)]
pub struct Identify {
    #[serde(rename = "rpcVersion")]
    pub rpc_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<String>,
    #[serde(rename = "eventSubscriptions")]
    pub event_subscriptions: u32,
}

#[derive(Debug, Deserialize)]
pub struct Identified {
    #[serde(rename = "negotiatedRpcVersion")]
    pub negotiated_rpc_version: u32,
}

#[derive(Debug, Serialize)]
pub struct Request {
    #[serde(rename = "requestType")]
    pub request_type: String,
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "requestData", skip_serializing_if = "Option::is_none")]
    pub request_data: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct RequestBatch {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "haltOnFailure")]
    pub halt_on_failure: bool,
    pub requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
pub struct RequestResponse {
    #[serde(rename = "requestType")]
    pub request_type: String,
    #[serde(rename = "requestId", default)]
    pub request_id: String,
    #[serde(rename = "requestStatus")]
    pub request_status: RequestStatus,
    #[serde(rename = "responseData", default)]
    pub response_data: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct RequestBatchResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(default)]
    pub results: Vec<RequestResponse>,
}

#[derive(Debug, Deserialize)]
pub struct RequestStatus {
    pub result: bool,
    pub code: u32,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    #[serde(rename = "eventType")]
    pub event_type: String,
    #[serde(rename = "eventData", default)]
    pub event_data: Option<Value>,
}

/// obs-websocket status code for a rejected `Identify`.
pub const CODE_AUTH_FAILED: u32 = 4009;

impl RequestResponse {
    /// The response payload if the request succeeded, otherwise `None`.
    ///
    /// A batch member that fails (e.g. `GetStreamStatus` while no output
    /// exists) should degrade to a missing metric, not fail the whole poll.
    pub fn ok_data(&self) -> Option<&Value> {
        if self.request_status.result {
            self.response_data.as_ref()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_subscription_includes_volume_meters_and_all_base_events() {
        assert_eq!(events::ALL, 0x7FF);
        assert_eq!(events::DASHBOARD, 0x7FF | 0x10000);
        assert_ne!(events::DASHBOARD & events::INPUT_VOLUME_METERS, 0);
        assert_ne!(events::DASHBOARD & events::OUTPUTS, 0);
    }

    #[test]
    fn identify_omits_authentication_when_absent() {
        let identify = Identify {
            rpc_version: RPC_VERSION,
            authentication: None,
            event_subscriptions: events::DASHBOARD,
        };
        let json = serde_json::to_string(&identify).unwrap();
        assert!(!json.contains("authentication"));
        assert!(json.contains("\"rpcVersion\":1"));
        assert!(json.contains("eventSubscriptions"));
    }

    #[test]
    fn hello_parses_with_and_without_auth() {
        let anonymous: Hello =
            serde_json::from_str(r#"{"obsWebSocketVersion":"5.5.0","rpcVersion":1}"#).unwrap();
        assert!(anonymous.authentication.is_none());
        assert_eq!(anonymous.rpc_version, 1);

        let secured: Hello = serde_json::from_str(
            r#"{"obsWebSocketVersion":"5.5.0","rpcVersion":1,
                "authentication":{"challenge":"c","salt":"s"}}"#,
        )
        .unwrap();
        let auth = secured.authentication.expect("authentication present");
        assert_eq!(auth.salt, "s");
        assert_eq!(auth.challenge, "c");
    }

    #[test]
    fn failed_batch_member_yields_no_data() {
        let response: RequestResponse = serde_json::from_str(
            r#"{"requestType":"GetStreamStatus","requestId":"poll-1",
                "requestStatus":{"result":false,"code":604,"comment":"no output"}}"#,
        )
        .unwrap();
        assert!(response.ok_data().is_none());

        let ok: RequestResponse = serde_json::from_str(
            r#"{"requestType":"GetStats","requestId":"poll-1",
                "requestStatus":{"result":true,"code":100},
                "responseData":{"cpuUsage":3.5}}"#,
        )
        .unwrap();
        assert_eq!(ok.ok_data().unwrap()["cpuUsage"], 3.5);
    }

    #[test]
    fn batch_response_parses_mixed_results() {
        let batch: RequestBatchResponse = serde_json::from_str(
            r#"{"requestId":"poll-7","results":[
                {"requestType":"GetStats","requestStatus":{"result":true,"code":100},
                 "responseData":{"cpuUsage":1.0}},
                {"requestType":"GetStreamStatus","requestStatus":{"result":false,"code":604}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(batch.request_id, "poll-7");
        assert_eq!(batch.results.len(), 2);
        assert!(batch.results[0].ok_data().is_some());
        assert!(batch.results[1].ok_data().is_none());
    }
}

//! The background OBS session task.
//!
//! One task owns the socket for the whole process lifetime. It reconnects on
//! its own, polls on a timer, and pushes [`Update`] values at the UI over an
//! unbounded-enough channel. The UI never awaits OBS, so a hung or missing
//! OBS degrades to a stale-looking dashboard instead of a frozen terminal.
//!
//! Polling is split in two cadences because the payloads differ wildly in
//! cost. The hot batch (`GetStats` + output status) runs every tick; the
//! inventory batch (scenes, inputs, profiles, video settings) runs every
//! [`INVENTORY_EVERY`] ticks, since those change on human timescales.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Redacted, Result};
use crate::obs::auth::compute_authentication;
use crate::obs::protocol::{
    self, Envelope, Event, Hello, Identified, Identify, Request, RequestBatch,
    RequestBatchResponse, RequestResponse,
};
use crate::obs::stats::{
    AudioInput, RecordStatus, SceneList, Stats, StreamStatus, Version, VideoSettings, is_audio_kind,
};

/// How many hot polls pass between full inventory refreshes.
const INVENTORY_EVERY: u32 = 10;

/// Reconnect backoff schedule, in seconds, saturating at the last entry.
const BACKOFF_SECONDS: [u64; 5] = [1, 2, 4, 8, 15];

/// Capacity of the update channel. Large enough that a slow frame never drops
/// a metric, small enough that a wedged UI cannot grow the queue unboundedly.
const UPDATE_CHANNEL_CAPACITY: usize = 256;

const REQUEST_ID_POLL: &str = "obs-stats-poll";
const REQUEST_ID_INVENTORY: &str = "obs-stats-inventory";
const REQUEST_ID_AUDIO: &str = "obs-stats-audio";
const REQUEST_ID_VERSION: &str = "obs-stats-version";

/// Everything needed to open a session. Debug-safe: the password is
/// [`Redacted`], so logging this struct cannot leak it.
#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub url: String,
    pub password: Option<Redacted>,
    pub connect_timeout: Duration,
    pub poll_interval: Duration,
}

/// Messages pushed from the session task to the UI.
#[derive(Debug, Clone)]
pub enum Update {
    /// A connection attempt has started. `attempt` counts from 1.
    Connecting { attempt: u32 },
    /// Handshake completed; OBS identified itself.
    Connected(Box<Version>),
    /// A hot poll landed.
    Poll(Box<PollData>),
    /// A slow inventory poll landed.
    Inventory(Box<Inventory>),
    /// Fresh peak levels, keyed by input name.
    Meters(Vec<(String, Vec<f32>)>),
    /// A notable OBS event, already rendered for the log view.
    Event { level: LogLevel, message: String },
    /// The session ended and will be retried.
    Disconnected {
        reason: String,
        hint: Option<&'static str>,
        retry_in: Duration,
    },
    /// The session ended and will *not* be retried.
    Fatal {
        reason: String,
        hint: Option<&'static str>,
    },
}

/// Commands the UI can send back to the session task.
#[derive(Debug, Clone)]
pub enum Command {
    /// Poll immediately instead of waiting for the next tick.
    RefreshNow,
    /// Change the hot poll cadence.
    SetPollInterval(Duration),
    /// Close the socket and end the task.
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Payload of a hot poll.
#[derive(Debug, Clone, Default)]
pub struct PollData {
    pub stats: Stats,
    pub stream: StreamStatus,
    pub record: RecordStatus,
}

/// Payload of a slow inventory poll.
#[derive(Debug, Clone, Default)]
pub struct Inventory {
    pub video: VideoSettings,
    pub scenes: SceneList,
    pub inputs: Vec<AudioInput>,
    pub profiles: Vec<String>,
    pub current_profile: String,
    pub collections: Vec<String>,
    pub current_collection: String,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Spawn the session task.
///
/// Returns the update stream and a command handle. Dropping the command
/// handle does not stop the task; send [`Command::Shutdown`] for that.
pub fn spawn(params: ConnectionParams) -> (mpsc::Receiver<Update>, mpsc::Sender<Command>) {
    let (update_tx, update_rx) = mpsc::channel(UPDATE_CHANNEL_CAPACITY);
    let (command_tx, command_rx) = mpsc::channel(16);
    tokio::spawn(run(params, update_tx, command_rx));
    (update_rx, command_tx)
}

/// Reconnect loop. Runs until shutdown, a non-retryable error, or the UI
/// dropping the update receiver.
async fn run(
    mut params: ConnectionParams,
    update_tx: mpsc::Sender<Update>,
    mut command_rx: mpsc::Receiver<Command>,
) {
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        if update_tx
            .send(Update::Connecting { attempt })
            .await
            .is_err()
        {
            return; // UI is gone.
        }

        match session(&params, &update_tx, &mut command_rx).await {
            Ok(Outcome::Shutdown) => return,
            Ok(Outcome::IntervalChanged(interval)) => {
                // Restart the session so the new cadence takes effect from a
                // clean ticker; keep the attempt counter, it is only cosmetic.
                params.poll_interval = interval;
                attempt = attempt.saturating_sub(1);
                continue;
            }
            Err(error) => {
                let hint = error.hint();
                if !error.is_retryable() {
                    let _ = update_tx
                        .send(Update::Fatal {
                            reason: error.to_string(),
                            hint,
                        })
                        .await;
                    return;
                }

                let retry_in = backoff(attempt);
                let sent = update_tx
                    .send(Update::Disconnected {
                        reason: error.to_string(),
                        hint,
                        retry_in,
                    })
                    .await;
                if sent.is_err() {
                    return;
                }

                // Stay responsive to Shutdown while backing off.
                tokio::select! {
                    _ = tokio::time::sleep(retry_in) => {}
                    command = command_rx.recv() => match command {
                        Some(Command::Shutdown) | None => return,
                        Some(Command::SetPollInterval(interval)) => params.poll_interval = interval,
                        Some(Command::RefreshNow) => {}
                    }
                }
            }
        }
    }
}

/// Why a session ended without an error.
enum Outcome {
    Shutdown,
    IntervalChanged(Duration),
}

/// Backoff for the Nth consecutive failed attempt, saturating at the tail of
/// [`BACKOFF_SECONDS`].
fn backoff(attempt: u32) -> Duration {
    let index = (attempt.saturating_sub(1) as usize).min(BACKOFF_SECONDS.len() - 1);
    Duration::from_secs(BACKOFF_SECONDS[index])
}

/// Open one session and pump it until it fails or is told to stop.
async fn session(
    params: &ConnectionParams,
    update_tx: &mpsc::Sender<Update>,
    command_rx: &mut mpsc::Receiver<Command>,
) -> Result<Outcome> {
    let mut socket = connect(params).await?;
    let version = handshake(&mut socket, params).await?;

    if update_tx
        .send(Update::Connected(Box::new(version)))
        .await
        .is_err()
    {
        return Ok(Outcome::Shutdown);
    }

    let mut poll_ticker = tokio::time::interval(params.poll_interval);
    poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut inventory_due = 0_u32;

    loop {
        tokio::select! {
            // Bias the socket branch so a burst of meter events cannot be
            // starved by the poll timer.
            biased;

            message = socket.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        return Err(Error::ConnectionFailed {
                            url: params.url.clone(),
                            reason: error.to_string(),
                        });
                    }
                    None => return Err(Error::Disconnected),
                };

                match message {
                    Message::Text(text) => handle_frame(&text, update_tx, &mut socket).await?,
                    Message::Binary(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => handle_frame(&text, update_tx, &mut socket).await?,
                        Err(_) => return Err(Error::Protocol("non-UTF-8 frame".into())),
                    },
                    Message::Close(_) => return Err(Error::Disconnected),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }

            _ = poll_ticker.tick() => {
                send_json(&mut socket, &hot_poll_batch()).await?;
                if inventory_due == 0 {
                    send_json(&mut socket, &inventory_batch()).await?;
                }
                inventory_due = (inventory_due + 1) % INVENTORY_EVERY;
            }

            command = command_rx.recv() => match command {
                Some(Command::RefreshNow) => {
                    send_json(&mut socket, &hot_poll_batch()).await?;
                    send_json(&mut socket, &inventory_batch()).await?;
                    inventory_due = 1;
                }
                Some(Command::SetPollInterval(interval)) => {
                    let _ = socket.close(None).await;
                    return Ok(Outcome::IntervalChanged(interval));
                }
                Some(Command::Shutdown) | None => {
                    let _ = socket.close(None).await;
                    return Ok(Outcome::Shutdown);
                }
            },
        }
    }
}

async fn connect(params: &ConnectionParams) -> Result<Socket> {
    let (socket, _response) = tokio::time::timeout(
        params.connect_timeout,
        tokio_tungstenite::connect_async(&params.url),
    )
    .await
    .map_err(|_| Error::ConnectionFailed {
        url: params.url.clone(),
        reason: format!("timed out after {}ms", params.connect_timeout.as_millis()),
    })?
    .map_err(|error| Error::ConnectionFailed {
        url: params.url.clone(),
        reason: error.to_string(),
    })?;
    Ok(socket)
}

/// Perform the Hello → Identify → Identified exchange and fetch `GetVersion`.
async fn handshake(socket: &mut Socket, params: &ConnectionParams) -> Result<Version> {
    let hello: Hello = serde_json::from_value(expect_op(socket, protocol::OP_HELLO).await?)
        .map_err(|error| Error::Protocol(format!("malformed Hello: {error}")))?;

    let authentication = match (&hello.authentication, &params.password) {
        (Some(challenge), Some(password)) => Some(compute_authentication(
            password.expose(),
            &challenge.salt,
            &challenge.challenge,
        )),
        // OBS demands auth but we have no password: fail now with a clear
        // message rather than letting the server close with code 4009.
        (Some(_), None) => return Err(Error::AuthRequired),
        // A password configured against an unauthenticated server is
        // harmless — OBS ignores the field — so just omit it.
        (None, _) => None,
    };

    send_json(
        socket,
        &Envelope {
            op: protocol::OP_IDENTIFY,
            d: serde_json::to_value(Identify {
                rpc_version: protocol::RPC_VERSION,
                authentication,
                event_subscriptions: protocol::events::DASHBOARD,
            })
            .map_err(|error| Error::Protocol(error.to_string()))?,
        },
    )
    .await?;

    let identified: Identified =
        serde_json::from_value(expect_op(socket, protocol::OP_IDENTIFIED).await?)
            .map_err(|error| Error::Protocol(format!("malformed Identified: {error}")))?;
    if identified.negotiated_rpc_version != protocol::RPC_VERSION {
        return Err(Error::Protocol(format!(
            "OBS negotiated RPC version {} but this build speaks {}",
            identified.negotiated_rpc_version,
            protocol::RPC_VERSION
        )));
    }

    // Ask for the version banner, but do not block the dashboard on it: fall
    // back to what Hello already told us if the reply is slow or absent.
    send_json(
        socket,
        &Envelope {
            op: protocol::OP_REQUEST,
            d: json!({"requestType": "GetVersion", "requestId": REQUEST_ID_VERSION}),
        },
    )
    .await?;

    Ok(Version {
        websocket_version: hello.obs_websocket_version,
        ..Version::default()
    })
}

/// Read frames until one carries `op`, so a stray event during the handshake
/// does not abort the connection.
async fn expect_op(socket: &mut Socket, op: u8) -> Result<Value> {
    loop {
        let message = socket
            .next()
            .await
            .ok_or(Error::Disconnected)?
            .map_err(|error| Error::Protocol(error.to_string()))?;

        let text = match message {
            Message::Text(text) => text,
            Message::Binary(bytes) => {
                String::from_utf8(bytes).map_err(|_| Error::Protocol("non-UTF-8 frame".into()))?
            }
            // OBS closes with 4009 when Identify carries a bad authentication
            // string; surface that as a credential failure, not a disconnect.
            Message::Close(Some(frame))
                if u16::from(frame.code) == protocol::CODE_AUTH_FAILED as u16 =>
            {
                return Err(Error::AuthFailed);
            }
            Message::Close(_) => return Err(Error::Disconnected),
            _ => continue,
        };

        let envelope: Envelope = serde_json::from_str(&text)
            .map_err(|error| Error::Protocol(format!("malformed frame: {error}")))?;
        if envelope.op == op {
            return Ok(envelope.d);
        }
    }
}

/// Decode one frame and, when it carries data, push an [`Update`].
async fn handle_frame(
    text: &str,
    update_tx: &mpsc::Sender<Update>,
    socket: &mut Socket,
) -> Result<()> {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(envelope) => envelope,
        // A frame we cannot parse is not worth killing the session over.
        Err(_) => return Ok(()),
    };

    match envelope.op {
        protocol::OP_REQUEST_BATCH_RESPONSE => {
            let batch: RequestBatchResponse = match serde_json::from_value(envelope.d) {
                Ok(batch) => batch,
                Err(_) => return Ok(()),
            };
            match batch.request_id.as_str() {
                REQUEST_ID_POLL => {
                    emit(
                        update_tx,
                        Update::Poll(Box::new(decode_poll(&batch.results))),
                    )
                    .await
                }
                REQUEST_ID_INVENTORY => {
                    let inventory = decode_inventory(&batch.results);
                    // The input list only arrives here, so the volume/mute
                    // lookups have to be issued as a follow-up batch.
                    if !inventory.inputs.is_empty() {
                        send_json(socket, &audio_batch(&inventory.inputs)).await?;
                    }
                    emit(update_tx, Update::Inventory(Box::new(inventory))).await;
                }
                REQUEST_ID_AUDIO => {
                    if let Some(levels) = decode_audio_levels(&batch.results) {
                        emit(update_tx, Update::Meters(levels)).await;
                    }
                }
                _ => {}
            }
        }
        protocol::OP_REQUEST_RESPONSE => {
            let response: RequestResponse = match serde_json::from_value(envelope.d) {
                Ok(response) => response,
                Err(_) => return Ok(()),
            };
            if response.request_id == REQUEST_ID_VERSION
                && let Some(data) = response.ok_data()
            {
                emit(
                    update_tx,
                    Update::Connected(Box::new(Version::from_value(data))),
                )
                .await;
            }
        }
        protocol::OP_EVENT => {
            let event: Event = match serde_json::from_value(envelope.d) {
                Ok(event) => event,
                Err(_) => return Ok(()),
            };
            handle_event(event, update_tx).await;
        }
        _ => {}
    }

    Ok(())
}

async fn handle_event(event: Event, update_tx: &mpsc::Sender<Update>) {
    let data = event.event_data.unwrap_or(Value::Null);

    // Volume meters are the only high-rate event; route them straight to the
    // meters channel instead of the log, or the log would be unreadable.
    if event.event_type == "InputVolumeMeters" {
        if let Some(levels) = decode_meter_event(&data) {
            emit(update_tx, Update::Meters(levels)).await;
        }
        return;
    }

    if let Some((level, message)) = describe_event(&event.event_type, &data) {
        emit(update_tx, Update::Event { level, message }).await;
    }
}

/// Turn an OBS event into a log line, or `None` for events too noisy to show.
///
/// Kept free of I/O so the mapping can be unit-tested directly.
pub fn describe_event(event_type: &str, data: &Value) -> Option<(LogLevel, String)> {
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

async fn emit(update_tx: &mpsc::Sender<Update>, update: Update) {
    let _ = update_tx.send(update).await;
}

async fn send_json<T: serde::Serialize>(socket: &mut Socket, payload: &T) -> Result<()> {
    let text =
        serde_json::to_string(payload).map_err(|error| Error::Protocol(error.to_string()))?;
    // A failed write means the peer went away mid-session. Reporting that as
    // ConnectionFailed would print "could not connect to OBS at :" — the URL
    // is not available here, and it is the wrong thing to say anyway.
    socket
        .send(Message::Text(text))
        .await
        .map_err(|error| Error::ConnectionLost(error.to_string()))
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
pub fn hot_poll_batch() -> Envelope {
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
pub fn inventory_batch() -> Envelope {
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
pub fn audio_batch(inputs: &[AudioInput]) -> Envelope {
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
pub fn decode_poll(results: &[RequestResponse]) -> PollData {
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
pub fn decode_inventory(results: &[RequestResponse]) -> Inventory {
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
pub fn decode_meter_event(data: &Value) -> Option<Vec<(String, Vec<f32>)>> {
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
fn decode_audio_levels(results: &[RequestResponse]) -> Option<Vec<(String, Vec<f32>)>> {
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
    fn backoff_grows_then_saturates() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(5), Duration::from_secs(15));
        assert_eq!(backoff(500), Duration::from_secs(15));
        // Attempt 0 should not underflow into the last slot.
        assert_eq!(backoff(0), Duration::from_secs(1));
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

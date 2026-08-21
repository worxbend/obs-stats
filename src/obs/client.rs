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
use crate::obs::codec::{
    REQUEST_ID_AUDIO, REQUEST_ID_INVENTORY, REQUEST_ID_POLL, REQUEST_ID_VERSION, audio_batch,
    decode_audio_levels, decode_inventory, decode_meter_event, decode_poll, describe_event,
    hot_poll_batch, inventory_batch,
};
use crate::obs::protocol::{
    self, Envelope, Event, Hello, Identified, Identify, RequestBatchResponse, RequestResponse,
};
use crate::obs::stats::{
    AudioInput, RecordStatus, SceneList, Stats, StreamStatus, Version, VideoSettings,
};

/// How many hot polls pass between full inventory refreshes.
const INVENTORY_EVERY: u32 = 10;

/// Reconnect backoff schedule, in seconds, saturating at the last entry.
const BACKOFF_SECONDS: [u64; 5] = [1, 2, 4, 8, 15];

/// Capacity of the update channel. Large enough that a slow frame never drops
/// a metric, small enough that a wedged UI cannot grow the queue unboundedly.
const UPDATE_CHANNEL_CAPACITY: usize = 256;

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
    /// Fresh peak levels from the high-rate `InputVolumeMeters` event.
    Meters(Vec<MeterFrame>),
    /// Mute and volume, from the follow-up batch issued after an inventory
    /// poll. A separate message from [`Update::Meters`] because it carries
    /// entirely different values on an entirely different cadence.
    Faders(Vec<FaderState>),
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

/// One input's peak level per channel, as linear magnitudes in `0.0..=1.0`.
///
/// Arrives many times a second while audio is flowing.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterFrame {
    pub name: String,
    pub peaks: Vec<f32>,
}

/// One input's mixer position: whether it is muted and where its fader sits.
///
/// Arrives once per inventory poll. Either field is `None` when OBS did not
/// answer that half of the pair — the decoders are total, so a missing field
/// means "unknown", not "zero".
#[derive(Debug, Clone, PartialEq)]
pub struct FaderState {
    pub name: String,
    pub muted: Option<bool>,
    pub volume_db: Option<f64>,
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
    // The websocket being open says nothing about OBS answering on it: a
    // proxy or a half-started OBS can accept the TCP connection and then
    // never send `Hello`. Without a deadline `handshake` would wait forever
    // and the dashboard would sit on "connecting" instead of failing over to
    // the retry loop, so give it the same budget as the connect itself.
    let version = tokio::time::timeout(params.connect_timeout, handshake(&mut socket, params))
        .await
        .map_err(|_| Error::ConnectionFailed {
            url: params.url.clone(),
            reason: format!(
                "handshake timed out after {}ms",
                params.connect_timeout.as_millis()
            ),
        })??;

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
                    if let Some(faders) = decode_audio_levels(&batch.results) {
                        emit(update_tx, Update::Faders(faders)).await;
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
        if let Some(frames) = decode_meter_event(&data) {
            emit(update_tx, Update::Meters(frames)).await;
        }
        return;
    }

    if let Some((level, message)) = describe_event(&event.event_type, &data) {
        emit(update_tx, Update::Event { level, message }).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

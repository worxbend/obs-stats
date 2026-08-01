//! obs-websocket 5.x client.
//!
//! [`client::spawn`] owns the socket on a background task and pushes
//! [`client::Update`] messages at the UI; the UI never awaits a round trip, so
//! a stalled OBS can never freeze a frame.

pub mod auth;
pub mod client;
pub mod protocol;
pub mod stats;

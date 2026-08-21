//! obs-websocket 5.x client.
//!
//! [`client::spawn`] owns the socket on a background task and pushes
//! [`client::Update`] messages at the UI; the UI never awaits a round trip, so
//! a stalled OBS can never freeze a frame.
//!
//! [`codec`] holds the other half of the protocol — building requests and
//! decoding responses — with no socket and no `async`, so the wire format can
//! be tested without a live OBS.

pub mod auth;
pub mod client;
pub(crate) mod codec;
pub mod protocol;
pub mod stats;

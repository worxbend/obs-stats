//! `obs-stats` — a btop-style terminal dashboard for OBS Studio.
//!
//! The crate is laid out as four layers, each of which can be tested without
//! the one above it:
//!
//! - [`obs`] speaks obs-websocket 5.x: handshake, request/response correlation,
//!   event subscriptions, and reconnection.
//! - [`metrics`] turns raw responses into a [`metrics::Snapshot`] plus rolling
//!   history buffers that the charts read from.
//! - [`config`] loads YAML settings and resolves the theme.
//! - [`tui`] renders. It is pure: given a snapshot and a theme it draws a
//!   frame, and never performs I/O of its own.

pub mod cli;
pub mod config;
pub mod error;
pub mod metrics;
pub mod notify;
pub mod obs;
pub mod probe;
pub mod tui;
pub mod util;

pub use error::{Error, Result};

/// Version string reported in the header and by `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product name shown in the TUI header and splash.
pub const APP_NAME: &str = "obs-stats";

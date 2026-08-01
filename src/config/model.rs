//! The configuration schema and its validation rules.
//!
//! Every field has a default, so an empty file — or no file at all — is a
//! valid configuration that talks to a stock OBS on localhost.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Bounds on the poll cadence.
///
/// Below 100 ms OBS spends more time answering than rendering; above a minute
/// the sparklines stop being live data.
pub const MIN_POLL_INTERVAL_MS: u64 = 100;
pub const MAX_POLL_INTERVAL_MS: u64 = 60_000;

/// Bounds on the retained history window, in seconds.
pub const MIN_HISTORY_SECONDS: u64 = 10;
pub const MAX_HISTORY_SECONDS: u64 = 3600;

/// Refuse absurd passwords early rather than hashing them.
pub const MAX_PASSWORD_LENGTH: usize = 512;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub connection: ConnectionConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    /// Name of the environment variable holding the OBS WebSocket password.
    ///
    /// The password itself is deliberately not a config field: this file is
    /// meant to be safe to commit and to paste into a bug report.
    pub password_env: String,
    pub connect_timeout_ms: u64,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4455,
            password_env: "OBS_WEBSOCKET_PASSWORD".to_string(),
            connect_timeout_ms: 3000,
        }
    }
}

impl ConnectionConfig {
    /// The `ws://host:port` URL this config points at.
    pub fn url(&self) -> String {
        // IPv6 literals need bracketing before the port separator.
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("ws://[{}]:{}", self.host, self.port)
        } else {
            format!("ws://{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Theme id, or `custom` when `custom_theme` is populated.
    pub theme: String,
    /// How often the hot metrics are polled.
    pub poll_interval_ms: u64,
    /// How much history the sparklines and charts retain.
    pub history_seconds: u64,
    /// Draw Unicode block glyphs and rounded borders. Disable for a TTY.
    pub unicode: bool,
    /// Show the emoji/glyph column in panels.
    pub show_icons: bool,
    /// Enable mouse selection and scrolling.
    pub mouse: bool,
    /// Which view is focused at startup.
    pub start_view: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "claude".to_string(),
            poll_interval_ms: 1000,
            history_seconds: 120,
            unicode: true,
            show_icons: true,
            mouse: true,
            start_view: "overview".to_string(),
        }
    }
}

impl Config {
    /// Check every invariant the rest of the program relies on.
    ///
    /// Returns the *first* problem found, phrased with the config key so the
    /// user knows exactly which line to edit.
    pub fn validate(&self) -> Result<()> {
        let connection = &self.connection;

        if connection.host.trim().is_empty() {
            return Err(Error::ConfigInvalid(
                "connection.host must not be empty".into(),
            ));
        }
        if connection
            .host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(Error::ConfigInvalid(
                "connection.host must not contain control or whitespace characters".into(),
            ));
        }
        if connection.host.contains('/') {
            return Err(Error::ConfigInvalid(
                "connection.host must be a bare host, not a URL".into(),
            ));
        }
        if connection.port == 0 {
            return Err(Error::ConfigInvalid(
                "connection.port must be in range 1-65535".into(),
            ));
        }
        if connection.connect_timeout_ms == 0 {
            return Err(Error::ConfigInvalid(
                "connection.connect_timeout_ms must be positive".into(),
            ));
        }
        if !connection.password_env.is_empty() && !is_env_name(&connection.password_env) {
            return Err(Error::ConfigInvalid(format!(
                "connection.password_env is not a valid environment variable name: {}",
                connection.password_env
            )));
        }

        let ui = &self.ui;
        if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&ui.poll_interval_ms) {
            return Err(Error::ConfigInvalid(format!(
                "ui.poll_interval_ms must be between {MIN_POLL_INTERVAL_MS} and {MAX_POLL_INTERVAL_MS}"
            )));
        }
        if !(MIN_HISTORY_SECONDS..=MAX_HISTORY_SECONDS).contains(&ui.history_seconds) {
            return Err(Error::ConfigInvalid(format!(
                "ui.history_seconds must be between {MIN_HISTORY_SECONDS} and {MAX_HISTORY_SECONDS}"
            )));
        }
        if crate::tui::theme::find(&ui.theme).is_none() {
            return Err(Error::ConfigInvalid(format!(
                "ui.theme '{}' is not a known theme; run `obs-stats themes` to list them",
                ui.theme
            )));
        }
        if crate::tui::view::View::from_id(&ui.start_view).is_none() {
            return Err(Error::ConfigInvalid(format!(
                "ui.start_view '{}' is not a known view",
                ui.start_view
            )));
        }

        Ok(())
    }
}

/// Whether `name` is a POSIX-shaped environment variable name.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default()
            .validate()
            .expect("defaults must validate");
    }

    #[test]
    fn defaults_point_at_a_stock_local_obs() {
        let config = Config::default();
        assert_eq!(config.connection.url(), "ws://127.0.0.1:4455");
        assert_eq!(config.connection.password_env, "OBS_WEBSOCKET_PASSWORD");
        assert_eq!(config.ui.theme, "claude");
    }

    #[test]
    fn ipv6_hosts_are_bracketed_in_the_url() {
        let config = ConnectionConfig {
            host: "::1".to_string(),
            ..ConnectionConfig::default()
        };
        assert_eq!(config.url(), "ws://[::1]:4455");

        let already = ConnectionConfig {
            host: "[::1]".to_string(),
            ..ConnectionConfig::default()
        };
        assert_eq!(already.url(), "ws://[::1]:4455");
    }

    fn invalid(mutate: impl FnOnce(&mut Config)) -> String {
        let mut config = Config::default();
        mutate(&mut config);
        config
            .validate()
            .expect_err("expected validation to fail")
            .to_string()
    }

    #[test]
    fn rejects_an_empty_host() {
        assert!(invalid(|c| c.connection.host = "  ".into()).contains("connection.host"));
    }

    #[test]
    fn rejects_a_host_that_is_really_a_url() {
        assert!(invalid(|c| c.connection.host = "ws://127.0.0.1".into()).contains("bare host"));
    }

    #[test]
    fn rejects_control_characters_in_the_host() {
        assert!(invalid(|c| c.connection.host = "local\nhost".into()).contains("control"));
    }

    #[test]
    fn rejects_a_zero_port_and_timeout() {
        assert!(invalid(|c| c.connection.port = 0).contains("connection.port"));
        assert!(invalid(|c| c.connection.connect_timeout_ms = 0).contains("connect_timeout_ms"));
    }

    #[test]
    fn rejects_a_malformed_password_env_name() {
        assert!(invalid(|c| c.connection.password_env = "1BAD".into()).contains("password_env"));
        assert!(
            invalid(|c| c.connection.password_env = "HAS SPACE".into()).contains("password_env")
        );
    }

    #[test]
    fn allows_an_empty_password_env_meaning_no_password() {
        let mut config = Config::default();
        config.connection.password_env = String::new();
        config.validate().expect("empty password_env is allowed");
    }

    #[test]
    fn rejects_out_of_range_poll_and_history() {
        assert!(invalid(|c| c.ui.poll_interval_ms = 10).contains("poll_interval_ms"));
        assert!(invalid(|c| c.ui.poll_interval_ms = 120_000).contains("poll_interval_ms"));
        assert!(invalid(|c| c.ui.history_seconds = 1).contains("history_seconds"));
        assert!(invalid(|c| c.ui.history_seconds = 99_999).contains("history_seconds"));
    }

    #[test]
    fn rejects_unknown_theme_and_view() {
        assert!(invalid(|c| c.ui.theme = "nope".into()).contains("not a known theme"));
        assert!(invalid(|c| c.ui.start_view = "nope".into()).contains("not a known view"));
    }

    #[test]
    fn env_name_shapes() {
        assert!(is_env_name("OBS_WEBSOCKET_PASSWORD"));
        assert!(is_env_name("_private"));
        assert!(is_env_name("A1"));
        assert!(!is_env_name(""));
        assert!(!is_env_name("1A"));
        assert!(!is_env_name("A-B"));
        assert!(!is_env_name("A B"));
    }

    #[test]
    fn round_trips_through_yaml() {
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn partial_yaml_fills_in_defaults() {
        let parsed: Config = serde_yaml::from_str("ui:\n  theme: nord\n").unwrap();
        assert_eq!(parsed.ui.theme, "nord");
        assert_eq!(parsed.ui.poll_interval_ms, 1000);
        assert_eq!(parsed.connection, ConnectionConfig::default());
    }

    #[test]
    fn unknown_keys_are_rejected_rather_than_silently_ignored() {
        let error = serde_yaml::from_str::<Config>("ui:\n  thmee: nord\n")
            .expect_err("typo must not be silently accepted");
        assert!(error.to_string().contains("thmee"), "{error}");
    }
}

//! Command-line interface.
//!
//! Flags override the config file for the current run only; nothing here
//! writes back to disk except the explicit `init` subcommand.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::config::model::{MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS};
use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::obs::client::ConnectionParams;
use crate::tui::theme;
use crate::tui::view::View;

#[derive(Debug, Parser)]
#[command(
    name = "obs-stats",
    version,
    about = "A terminal-native observability dashboard for OBS Studio",
    long_about = "obs-stats connects to OBS Studio over obs-websocket 5.x and renders a live \
                  terminal dashboard: CPU and memory, frame pacing, stream and recording \
                  outputs, scenes, and audio meters.\n\n\
                  With no subcommand it launches the dashboard.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<SubCommand>,

    /// Path to the config file. Defaults to $OBS_STATS_CONFIG, then
    /// ~/.config/obs-stats/config.yml.
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,

    /// Host running obs-websocket.
    #[arg(long, value_name = "HOST", global = true)]
    pub host: Option<String>,

    /// Port obs-websocket listens on.
    #[arg(long, value_name = "PORT", global = true)]
    pub port: Option<u16>,

    /// Environment variable to read the OBS password from.
    #[arg(long, value_name = "NAME", global = true)]
    pub password_env: Option<String>,

    /// Theme id. Run `obs-stats themes` for the full list.
    #[arg(long, value_name = "ID", global = true)]
    pub theme: Option<String>,

    /// Hot-poll interval in milliseconds.
    #[arg(long, value_name = "MS", global = true)]
    pub interval: Option<u64>,

    /// View to open on. One of: overview, performance, stream, scenes, audio, log.
    #[arg(long, value_name = "VIEW", global = true)]
    pub view: Option<String>,

    /// Disable Unicode drawing, for a Linux VT or a terminal without a good font.
    #[arg(long, global = true)]
    pub ascii: bool,

    /// Disable mouse capture.
    #[arg(long, global = true)]
    pub no_mouse: bool,

    /// Disable frame-loss alerts entirely (banner and notifications).
    #[arg(long, global = true)]
    pub no_alerts: bool,

    /// Keep the in-app alert banner but send no desktop notifications.
    #[arg(long, global = true)]
    pub no_notify: bool,

    /// Write debug logs to this file. Never contains the password.
    #[arg(long, value_name = "PATH", global = true)]
    pub log_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum SubCommand {
    /// Write a commented starter config and exit.
    Init,
    /// List the built-in themes and exit.
    Themes,
    /// Print the effective configuration and exit.
    Config,
    /// Connect, print one snapshot of the stats as JSON, and exit.
    Once,
}

impl Cli {
    /// The config file this invocation should read.
    pub fn config_path(&self) -> PathBuf {
        self.config.clone().unwrap_or_else(config::config_path)
    }

    /// Load the config file and layer the CLI flags on top.
    pub fn resolve(&self) -> Result<Config> {
        let mut config = config::load(&self.config_path())?;

        if let Some(host) = &self.host {
            config.connection.host = host.clone();
        }
        if let Some(port) = self.port {
            config.connection.port = port;
        }
        if let Some(name) = &self.password_env {
            config.connection.password_env = name.clone();
        }
        if let Some(id) = &self.theme {
            // Resolve here rather than at render time so a typo is a clear
            // startup error instead of a silent fallback to the default.
            let resolved = theme::find(id).ok_or_else(|| {
                Error::ConfigInvalid(format!(
                    "--theme '{id}' is not a known theme; run `obs-stats themes` to list them"
                ))
            })?;
            config.ui.theme = resolved.id.to_string();
        }
        if let Some(interval) = self.interval {
            if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&interval) {
                return Err(Error::ConfigInvalid(format!(
                    "--interval must be between {MIN_POLL_INTERVAL_MS} and {MAX_POLL_INTERVAL_MS} ms"
                )));
            }
            config.ui.poll_interval_ms = interval;
        }
        if let Some(view) = &self.view {
            let resolved = View::from_id(view).ok_or_else(|| {
                Error::ConfigInvalid(format!(
                    "--view '{view}' is not a known view; expected one of: {}",
                    View::ALL
                        .iter()
                        .map(|view| view.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            config.ui.start_view = resolved.id().to_string();
        }
        if self.ascii {
            config.ui.unicode = false;
            config.ui.show_icons = false;
        }
        if self.no_mouse {
            config.ui.mouse = false;
        }
        if self.no_alerts {
            config.alerts.enabled = false;
        }
        if self.no_notify {
            config.alerts.desktop = false;
        }

        // Re-validate: a flag can push a field out of range just as a file can.
        config.validate()?;
        Ok(config)
    }

    /// Build the OBS connection parameters for a resolved config.
    pub fn connection(config: &Config) -> Result<ConnectionParams> {
        Ok(ConnectionParams {
            url: config.connection.url(),
            password: config::resolve_password(&config.connection.password_env)?,
            connect_timeout: Duration::from_millis(config.connection.connect_timeout_ms),
            poll_interval: Duration::from_millis(config.ui.poll_interval_ms),
        })
    }
}

/// Render the theme list for `obs-stats themes`.
pub fn theme_list(active: &str) -> String {
    let mut out = String::from("Built-in themes:\n\n");
    for entry in theme::THEMES {
        let marker = if entry.id == active { "*" } else { " " };
        out.push_str(&format!("  {marker} {:<18} {}\n", entry.id, entry.label));
    }
    out.push_str("\nSet one with `ui.theme` in the config, `--theme <id>`, or F2 in the app.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("obs-stats").chain(args.iter().copied()))
    }

    /// Resolve against a temp config dir so the developer's own config never
    /// leaks into the assertions.
    fn resolve(args: &[&str]) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        let mut cli = parse(args);
        cli.config = Some(path);
        cli.resolve()
    }

    #[test]
    fn the_cli_definition_is_internally_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_means_launch_the_dashboard() {
        let cli = parse(&[]);
        assert!(cli.command.is_none());
        assert!(!cli.ascii);
        assert!(!cli.no_mouse);
    }

    #[test]
    fn subcommands_parse() {
        assert!(matches!(parse(&["init"]).command, Some(SubCommand::Init)));
        assert!(matches!(
            parse(&["themes"]).command,
            Some(SubCommand::Themes)
        ));
        assert!(matches!(
            parse(&["config"]).command,
            Some(SubCommand::Config)
        ));
        assert!(matches!(parse(&["once"]).command, Some(SubCommand::Once)));
    }

    #[test]
    fn defaults_survive_when_no_flags_are_given() {
        let config = resolve(&[]).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn connection_flags_override_the_file() {
        let config = resolve(&["--host", "192.168.1.10", "--port", "4466"]).unwrap();
        assert_eq!(config.connection.host, "192.168.1.10");
        assert_eq!(config.connection.port, 4466);
        assert_eq!(config.connection.url(), "ws://192.168.1.10:4466");
    }

    #[test]
    fn the_password_env_name_can_be_overridden() {
        let config = resolve(&["--password-env", "MY_OBS_PW"]).unwrap();
        assert_eq!(config.connection.password_env, "MY_OBS_PW");
    }

    #[test]
    fn a_theme_flag_is_normalised_and_validated() {
        let config = resolve(&["--theme", "Tokyo_Night"]).unwrap();
        assert_eq!(config.ui.theme, "tokyo-night");

        let error = resolve(&["--theme", "nope"]).unwrap_err().to_string();
        assert!(error.contains("not a known theme"), "{error}");
        assert!(
            error.contains("obs-stats themes"),
            "the error must say how to list them"
        );
    }

    #[test]
    fn a_view_flag_is_validated_and_lists_the_alternatives() {
        assert_eq!(
            resolve(&["--view", "audio"]).unwrap().ui.start_view,
            "audio"
        );

        let error = resolve(&["--view", "nope"]).unwrap_err().to_string();
        assert!(error.contains("not a known view"));
        assert!(error.contains("performance"), "{error}");
    }

    #[test]
    fn the_interval_flag_is_range_checked() {
        assert_eq!(
            resolve(&["--interval", "250"]).unwrap().ui.poll_interval_ms,
            250
        );
        assert!(resolve(&["--interval", "5"]).is_err());
        assert!(resolve(&["--interval", "999999"]).is_err());
    }

    #[test]
    fn ascii_mode_turns_off_both_unicode_and_icons() {
        let config = resolve(&["--ascii"]).unwrap();
        assert!(!config.ui.unicode);
        assert!(!config.ui.show_icons);
    }

    #[test]
    fn the_mouse_can_be_disabled() {
        assert!(!resolve(&["--no-mouse"]).unwrap().ui.mouse);
        assert!(resolve(&[]).unwrap().ui.mouse);
    }

    #[test]
    fn alerts_can_be_narrowed_or_silenced() {
        let quiet = resolve(&["--no-notify"]).unwrap();
        assert!(quiet.alerts.enabled, "the banner should remain");
        assert!(!quiet.alerts.desktop);

        let silent = resolve(&["--no-alerts"]).unwrap();
        assert!(!silent.alerts.enabled);

        let default = resolve(&[]).unwrap();
        assert!(default.alerts.enabled && default.alerts.desktop);
    }

    #[test]
    fn flags_layer_on_top_of_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "ui:\n  theme: nord\n  poll_interval_ms: 2000\n").unwrap();

        let mut cli = parse(&["--interval", "500"]);
        cli.config = Some(path);
        let config = cli.resolve().unwrap();

        assert_eq!(config.ui.theme, "nord", "unflagged file values survive");
        assert_eq!(config.ui.poll_interval_ms, 500, "the flag wins");
    }

    #[test]
    fn global_flags_work_alongside_a_subcommand() {
        let cli = parse(&["config", "--theme", "nord"]);
        assert!(matches!(cli.command, Some(SubCommand::Config)));
        assert_eq!(cli.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn connection_params_never_expose_the_password_in_debug_output() {
        let config = Config::default();
        let params = Cli::connection(&config).unwrap();
        assert_eq!(params.url, "ws://127.0.0.1:4455");
        assert!(!format!("{params:?}").contains("hunter"));
        assert_eq!(params.poll_interval, Duration::from_millis(1000));
    }

    #[test]
    fn the_theme_listing_covers_every_theme_and_marks_the_active_one() {
        let listing = theme_list("nord");
        for entry in theme::THEMES {
            assert!(listing.contains(entry.id), "{} is missing", entry.id);
            assert!(listing.contains(entry.label));
        }
        assert!(listing.contains("* nord"));
    }
}

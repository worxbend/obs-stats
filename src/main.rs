//! Entry point: parse the CLI, dispatch a subcommand, or launch the dashboard.

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser as _;

use obs_stats::cli::{Cli, SubCommand, theme_list};
use obs_stats::config;
use obs_stats::error::{Error, Result};
use obs_stats::probe;
use obs_stats::report;
use obs_stats::tui;

/// How long `once` waits for a full picture before printing what it has.
const ONCE_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli);

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The TUI has already restored the terminal by this point, so a
            // plain stderr line is readable.
            eprintln!("obs-stats: {error}");
            if let Some(hint) = error.hint() {
                eprintln!("  {hint}");
            }
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(SubCommand::Init) => init(&cli),
        Some(SubCommand::Themes) => {
            let config = cli.resolve()?;
            print!("{}", theme_list(&config.ui.theme));
            Ok(())
        }
        Some(SubCommand::Config) => dump_config(&cli),
        Some(SubCommand::Once) => runtime()?.block_on(once(&cli)),
        None => runtime()?.block_on(dashboard(cli)),
    }
}

/// A multi-threaded runtime so the render loop and the socket never queue
/// behind each other on a single worker.
fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| Error::Terminal(format!("could not start the async runtime: {error}")))
}

async fn dashboard(cli: Cli) -> Result<()> {
    let config = cli.resolve()?;
    let params = Cli::connection(&config)?;
    tui::run::run(config, params).await
}

fn init(cli: &Cli) -> Result<()> {
    let path = cli.config_path();
    config::write_default(&path)?;
    println!("Wrote {}", path.display());
    println!();
    println!("Set your OBS WebSocket password in the environment before starting:");
    println!("  export OBS_WEBSOCKET_PASSWORD='...'");
    Ok(())
}

fn dump_config(cli: &Cli) -> Result<()> {
    let config = cli.resolve()?;
    let path = cli.config_path();

    println!("# effective configuration");
    println!("# file: {}", path.display());
    if !path.exists() {
        println!("# (not present — these are the built-in defaults)");
    }
    // Report whether the secret is *available*, never what it is.
    let password = config::resolve_password(&config.connection.password_env)?;
    println!(
        "# password: {}",
        match (&password, config.connection.password_env.is_empty()) {
            (Some(_), _) => format!("set via ${}", config.connection.password_env),
            (None, true) => "not configured (anonymous)".to_string(),
            (None, false) => format!("${} is unset", config.connection.password_env),
        }
    );
    println!();

    let yaml =
        serde_yaml::to_string(&config).map_err(|error| Error::ConfigInvalid(error.to_string()))?;
    print!("{yaml}");
    Ok(())
}

/// Connect, collect one full picture, print it as JSON, and exit.
///
/// Useful for scripting and for bug reports: the same numbers the dashboard
/// draws, in a form a shell can consume.
async fn once(cli: &Cli) -> Result<()> {
    let config = cli.resolve()?;
    let params = Cli::connection(&config)?;
    let snapshot = probe::collect(params, ONCE_TIMEOUT).await?;

    let json = serde_json::to_string_pretty(&report::as_json(&snapshot))
        .map_err(|error| Error::Protocol(error.to_string()))?;
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// Set up file logging when `--log-file` is given.
///
/// Logging is off by default: a TUI cannot share stdout with a logger, and a
/// dashboard nobody asked to debug should not be writing files.
fn init_logging(cli: &Cli) {
    let Some(path) = &cli.log_file else { return };

    let Some(parent) = path.parent() else { return };
    if !parent.as_os_str().is_empty() && std::fs::create_dir_all(parent).is_err() {
        return;
    }

    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };

    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_env("OBS_STATS_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("obs_stats=debug"));

    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file)
                .with_ansi(false),
        )
        .with(filter)
        .try_init();
}

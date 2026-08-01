//! Entry point: parse the CLI, dispatch a subcommand, or launch the dashboard.

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser as _;

use obs_stats::cli::{Cli, SubCommand, theme_list};
use obs_stats::config;
use obs_stats::error::{Error, Result};
use obs_stats::metrics::Snapshot;
use obs_stats::probe;
use obs_stats::{tui, util};

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

    let json = serde_json::to_string_pretty(&as_json(&snapshot))
        .map_err(|error| Error::Protocol(error.to_string()))?;
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{json}")?;
    Ok(())
}

/// The documented JSON shape of `obs-stats once`.
fn as_json(snapshot: &Snapshot) -> serde_json::Value {
    let stats = &snapshot.stats;
    serde_json::json!({
        "obs": {
            "version": snapshot.version.obs_version,
            "websocket_version": snapshot.version.websocket_version,
            "platform": snapshot.version.platform,
        },
        "system": {
            "cpu_percent": stats.cpu_usage,
            "memory_mib": stats.memory_usage,
            "disk_free_mib": stats.available_disk_space,
        },
        "video": {
            "active_fps": stats.active_fps,
            "target_fps": snapshot.video.target_fps(),
            "average_frame_render_ms": stats.average_frame_render_time,
            "frame_budget_used": snapshot.frame_budget_ratio(),
            "base": format!("{}x{}", snapshot.video.base_width, snapshot.video.base_height),
            "output": format!("{}x{}", snapshot.video.output_width, snapshot.video.output_height),
        },
        "frames": {
            "render_skipped": stats.render_skipped_frames,
            "render_total": stats.render_total_frames,
            "output_skipped": stats.output_skipped_frames,
            "output_total": stats.output_total_frames,
        },
        "stream": {
            "active": snapshot.stream.active,
            "reconnecting": snapshot.stream.reconnecting,
            "duration_ms": snapshot.stream.duration_ms,
            "duration": util::duration_ms(snapshot.stream.duration_ms),
            "bytes": snapshot.stream.bytes,
            "congestion": snapshot.stream.congestion,
            "dropped_frames": snapshot.stream.skipped_frames,
            "total_frames": snapshot.stream.total_frames,
            "drop_percent": snapshot.stream.drop_percent(),
        },
        "record": {
            "active": snapshot.record.active,
            "paused": snapshot.record.paused,
            "duration_ms": snapshot.record.duration_ms,
            "duration": util::duration_ms(snapshot.record.duration_ms),
            "bytes": snapshot.record.bytes,
        },
        "scenes": {
            "program": snapshot.scenes.current_program,
            "preview": snapshot.scenes.current_preview,
            "all": snapshot.scenes.scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        },
        "profile": snapshot.current_profile,
        "collection": snapshot.current_collection,
        "inputs": snapshot.inputs.iter().map(|input| serde_json::json!({
            "name": input.name,
            "kind": input.kind,
            "muted": input.muted,
            "volume_db": if input.volume_db.is_finite() { Some(input.volume_db) } else { None },
        })).collect::<Vec<_>>(),
        "health": snapshot.health().label(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use obs_stats::obs::stats::{AudioInput, SceneList, Stats, StreamStatus, VideoSettings};

    fn populated() -> Snapshot {
        let mut snapshot = Snapshot::new(8);
        snapshot.stats = Stats {
            cpu_usage: 12.5,
            memory_usage: 1500.0,
            available_disk_space: 250_000.0,
            active_fps: 59.94,
            average_frame_render_time: 3.1,
            render_skipped_frames: 4,
            render_total_frames: 10_000,
            ..Stats::default()
        };
        snapshot.video = VideoSettings {
            base_width: 1920,
            base_height: 1080,
            output_width: 1280,
            output_height: 720,
            fps_numerator: 60,
            fps_denominator: 1,
        };
        snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 65_000,
            bytes: 1000,
            skipped_frames: 2,
            total_frames: 200,
            ..StreamStatus::default()
        };
        snapshot.scenes = SceneList {
            current_program: "Live".into(),
            ..SceneList::default()
        };
        snapshot.current_profile = "Twitch".into();
        snapshot.inputs = vec![AudioInput {
            name: "Mic".into(),
            kind: "pulse_input_capture".into(),
            muted: false,
            volume_db: -6.0,
            ..AudioInput::default()
        }];
        snapshot
    }

    #[test]
    fn json_output_covers_every_documented_section() {
        let json = as_json(&populated());
        for key in [
            "obs",
            "system",
            "video",
            "frames",
            "stream",
            "record",
            "scenes",
            "profile",
            "collection",
            "inputs",
            "health",
        ] {
            assert!(json.get(key).is_some(), "missing section {key}");
        }
    }

    #[test]
    fn json_output_carries_the_real_numbers() {
        let json = as_json(&populated());
        assert_eq!(json["system"]["cpu_percent"], 12.5);
        assert_eq!(json["video"]["active_fps"], 59.94);
        assert_eq!(json["video"]["target_fps"], 60.0);
        assert_eq!(json["video"]["base"], "1920x1080");
        assert_eq!(json["video"]["output"], "1280x720");
        assert_eq!(json["frames"]["render_skipped"], 4);
        assert_eq!(json["stream"]["active"], true);
        assert_eq!(json["stream"]["duration"], "01:05");
        assert_eq!(json["stream"]["drop_percent"], 1.0);
        assert_eq!(json["scenes"]["program"], "Live");
        assert_eq!(json["profile"], "Twitch");
    }

    #[test]
    fn json_output_lists_inputs_with_their_fader_positions() {
        let json = as_json(&populated());
        let inputs = json["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["name"], "Mic");
        assert_eq!(inputs[0]["muted"], false);
        assert_eq!(inputs[0]["volume_db"], -6.0);
    }

    #[test]
    fn an_unknown_fader_position_serialises_as_null_not_nan() {
        // serde_json cannot represent -inf, so it must be normalised away.
        let mut snapshot = populated();
        snapshot.inputs[0].volume_db = f64::NEG_INFINITY;
        let json = as_json(&snapshot);
        assert!(json["inputs"][0]["volume_db"].is_null());
        // The whole document must still serialise.
        serde_json::to_string(&json).expect("must serialise");
    }

    #[test]
    fn an_empty_snapshot_still_produces_valid_json() {
        let json = as_json(&Snapshot::new(8));
        let text = serde_json::to_string_pretty(&json).unwrap();
        assert!(text.contains("\"health\""));
        assert_eq!(json["stream"]["active"], false);
        assert_eq!(json["inputs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_health_verdict_is_reported_as_a_word() {
        let json = as_json(&populated());
        // 2 of 200 frames dropped is 1%, which is the "dropping" threshold.
        assert_eq!(json["health"], "DROPPING");
        assert_eq!(as_json(&Snapshot::new(8))["health"], "HEALTHY");
    }
}

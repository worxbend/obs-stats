//! Generate the documentation screenshots.
//!
//! Renders the real UI through ratatui's `TestBackend` and serialises the
//! resulting cell buffer to SVG, so the images in the README and on the
//! microsite are produced by the same code that draws the terminal — they
//! cannot drift from what the app actually looks like.
//!
//! ```text
//! cargo run --example screenshot                 # live OBS if reachable
//! cargo run --example screenshot -- --demo       # synthetic data only
//! cargo run --example screenshot -- --text       # dump plain text instead
//! cargo run --example screenshot -- --ascii      # ASCII fallback rendering
//! ```

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use obs_stats::config::Config;
use obs_stats::metrics::Snapshot;
use obs_stats::metrics::alerts::{Alert, AlertKind};
use obs_stats::obs::client::{LogLevel, Update};
use obs_stats::obs::stats::{
    AudioInput, RecordStatus, Scene, SceneList, Stats, StreamStatus, Version, VideoSettings,
};
use obs_stats::tui::app::App;
use obs_stats::tui::keys::{Action, Overlay, Pending};
use obs_stats::tui::theme::{self, Theme};
use obs_stats::tui::ui;
use obs_stats::tui::view::View;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// Terminal geometry the screenshots are taken at. 16:9-ish and wide enough
/// for the three-column decks, so the images show the intended layout.
const WIDTH: u16 = 132;
const HEIGHT: u16 = 38;

/// SVG cell metrics, tuned so box-drawing characters tile without seams.
const CELL_WIDTH: f32 = 8.4;
const CELL_HEIGHT: f32 = 18.0;
const FONT_SIZE: f32 = 14.0;
/// Distance from the top of a cell to the text baseline.
const BASELINE: f32 = 13.6;

/// Version label baked into the screenshots.
///
/// Pinned rather than taken from the crate: the header prints the version, so
/// letting the real one through would redraw every image on every release
/// bump and fail CI's no-drift check each time.
const SHOWN_VERSION: &str = "0.1.0";
/// Padding around the terminal grid, so the image reads as a window.
const PADDING: f32 = 12.0;

/// These are synthetic presentation fixtures, never recorded production data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Scenario {
    Healthy,
    Strained,
    Drops,
    Loading,
    Offline,
    Retrying,
    Paused,
    RecordingPaused,
    Stale,
}

impl Scenario {
    const ALL: [Self; 9] = [
        Self::Healthy,
        Self::Strained,
        Self::Drops,
        Self::Loading,
        Self::Offline,
        Self::Retrying,
        Self::Paused,
        Self::RecordingPaused,
        Self::Stale,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Strained => "strained",
            Self::Drops => "drops",
            Self::Loading => "loading",
            Self::Offline => "offline",
            Self::Retrying => "retrying",
            Self::Paused => "paused",
            Self::RecordingPaused => "recording-paused",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum TextMode {
    Grid,
    Summary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    width: u16,
    height: u16,
}

impl std::str::FromStr for Size {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (width, height) = value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
        let size = Self {
            width: width.parse().map_err(|_| "width must be an integer")?,
            height: height.parse().map_err(|_| "height must be an integer")?,
        };
        // Bound TestBackend allocations; even a maximum-size capture is ample
        // for layout inspection, without accepting accidental multi-GB grids.
        if size.width == 0 || size.height == 0 || size.width > 400 || size.height > 160 {
            return Err("size must be within 1x1 and 400x160".into());
        }
        Ok(size)
    }
}

const MATRIX_SIZES: [Size; 5] = [
    Size {
        width: 60,
        height: 18,
    },
    Size {
        width: 80,
        height: 24,
    },
    Size {
        width: 100,
        height: 30,
    },
    Size {
        width: WIDTH,
        height: HEIGHT,
    },
    Size {
        width: 180,
        height: 50,
    },
];

#[derive(Debug, Parser)]
#[command(about = "Export actual TUI frames; custom previews are always synthetic")]
struct Options {
    /// Preserve the deterministic documentation fixture (no network).
    #[arg(long)]
    demo: bool,
    /// Plain text grid, or a semantic summary for accessible inspection.
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "grid")]
    text: Option<TextMode>,
    /// Disable Unicode drawing and icons.
    #[arg(long)]
    ascii: bool,
    /// Terminal geometry; repeat to export several sizes.
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    size: Vec<Size>,
    /// Synthetic operating condition; repeat for several scenarios.
    #[arg(long, value_enum)]
    scenario: Vec<Scenario>,
    /// Export one view without overlays; repeat for several views.
    #[arg(long, value_parser = parse_view)]
    view: Vec<View>,
    /// Select a documentation shot, including its overlay; repeat as needed.
    #[arg(long, value_parser = parse_shot, conflicts_with = "view")]
    shot: Vec<String>,
    /// Theme id; repeat for several themes.
    #[arg(long, value_parser = parse_theme, conflicts_with = "all_themes")]
    theme: Vec<String>,
    /// Sweep every built-in theme, including mono.
    #[arg(long)]
    all_themes: bool,
    /// Sweep nine conditions and five terminal sizes (overrides their defaults).
    #[arg(long)]
    matrix: bool,
    /// Export raw cell symbols/colors/modifiers alongside SVGs.
    #[arg(long, conflicts_with = "text")]
    cells: bool,
    /// Output directory (custom previews default to docs/previews, not docs/images).
    #[arg(long, value_name = "DIR", conflicts_with = "text")]
    output: Option<PathBuf>,
}

impl Options {
    fn preview(&self) -> bool {
        self.matrix
            || self.cells
            || !self.size.is_empty()
            || !self.scenario.is_empty()
            || !self.view.is_empty()
            || !self.shot.is_empty()
            || !self.theme.is_empty()
            || self.all_themes
    }

    fn shots(&self) -> Vec<Shot> {
        if !self.view.is_empty() {
            return self
                .view
                .iter()
                .map(|&view| Shot {
                    name: view.id(),
                    view,
                    ..PLAIN
                })
                .collect();
        }
        SHOTS
            .iter()
            .filter(|shot| self.shot.is_empty() || self.shot.iter().any(|name| name == shot.name))
            .cloned()
            .collect()
    }
}

fn parse_view(value: &str) -> Result<View, String> {
    View::from_id(value).ok_or_else(|| format!("unknown view '{value}'"))
}

fn parse_shot(value: &str) -> Result<String, String> {
    SHOTS
        .iter()
        .any(|shot| shot.name == value)
        .then(|| value.to_string())
        .ok_or_else(|| format!("unknown shot '{value}'"))
}

fn parse_theme(value: &str) -> Result<String, String> {
    theme::find(value)
        .map(|theme| theme.id.to_string())
        .ok_or_else(|| format!("unknown theme '{value}'"))
}

#[derive(Clone)]
struct Shot {
    name: &'static str,
    view: View,
    overlay: Overlay,
    theme: &'static str,
    /// Show a frame-loss banner, for the alerts documentation.
    alert: Option<AlertKind>,
    /// A key sequence part-way through, which puts which-key on screen.
    pending: Pending,
    /// Text typed into the command line or the palette, whichever is open.
    typed: &'static str,
}

/// The defaults every shot starts from, so each entry below only states what
/// makes it different.
const PLAIN: Shot = Shot {
    name: "",
    view: View::Overview,
    overlay: Overlay::None,
    theme: "grafana",
    alert: None,
    pending: Pending::None,
    typed: "",
};

const SHOTS: &[Shot] = &[
    Shot {
        name: "overview",
        ..PLAIN
    },
    Shot {
        name: "performance",
        view: View::Performance,
        ..PLAIN
    },
    Shot {
        name: "stream",
        view: View::Stream,
        theme: "tokyo-night",
        ..PLAIN
    },
    Shot {
        name: "scenes",
        view: View::Scenes,
        theme: "nord",
        ..PLAIN
    },
    Shot {
        name: "audio",
        view: View::Audio,
        theme: "gruvbox",
        ..PLAIN
    },
    Shot {
        name: "log",
        view: View::Log,
        theme: "catppuccin-mocha",
        ..PLAIN
    },
    Shot {
        name: "themes",
        overlay: Overlay::Themes,
        theme: "dracula",
        ..PLAIN
    },
    Shot {
        name: "help",
        overlay: Overlay::Help,
        ..PLAIN
    },
    Shot {
        name: "alert",
        view: View::Stream,
        alert: Some(AlertKind::NetworkDropped),
        ..PLAIN
    },
    Shot {
        name: "whichkey",
        pending: Pending::Leader,
        ..PLAIN
    },
    Shot {
        name: "palette",
        overlay: Overlay::Palette,
        theme: "tokyo-night",
        typed: "the",
        ..PLAIN
    },
    Shot {
        name: "command",
        overlay: Overlay::Command,
        theme: "nord",
        typed: "theme no",
        ..PLAIN
    },
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Options::parse())
}

fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let preview = options.preview();
    let mut now = Instant::now() + Duration::from_secs(600);
    let (legacy, source) = if preview || options.demo {
        (
            demo_snapshot(now),
            "synthetic fixture; not production telemetry",
        )
    } else if let Some(live) = live_snapshot() {
        // Never dress live measurements with invented output states or history.
        now = Instant::now();
        (live, "live OBS measurements; no synthetic overlay")
    } else {
        (
            demo_snapshot(now),
            "synthetic fallback; no OBS reachable; not production telemetry",
        )
    };
    eprintln!("source: {source}");

    let out_dir = options.output.clone().unwrap_or_else(|| {
        PathBuf::from(if preview {
            "docs/previews"
        } else {
            "docs/images"
        })
    });
    if options.text.is_none() {
        std::fs::create_dir_all(&out_dir)?;
    }
    let sizes = if !options.size.is_empty() {
        options.size.clone()
    } else if options.matrix {
        MATRIX_SIZES.to_vec()
    } else {
        vec![Size {
            width: WIDTH,
            height: HEIGHT,
        }]
    };
    let scenarios: Vec<Option<Scenario>> = if !options.scenario.is_empty() {
        options.scenario.iter().copied().map(Some).collect()
    } else if options.matrix {
        Scenario::ALL.into_iter().map(Some).collect()
    } else if preview {
        vec![Some(Scenario::Healthy)]
    } else {
        vec![None]
    };
    let themes: Vec<String> = if options.all_themes {
        theme::THEMES
            .iter()
            .map(|theme| theme.id.to_string())
            .collect()
    } else {
        options.theme.clone()
    };
    let mut written = std::collections::HashSet::new();
    for scenario in scenarios {
        let snapshot = scenario.map_or_else(
            || legacy.clone(),
            |scenario| scenario_snapshot(scenario, now),
        );
        for size in &sizes {
            for shot in options.shots() {
                let shot_themes = if themes.is_empty() {
                    vec![shot.theme.to_string()]
                } else {
                    themes.clone()
                };
                for id in shot_themes {
                    let mut shot = shot.clone();
                    shot.theme = theme::resolve(&id).id;
                    let name = if preview {
                        format!(
                            "{}-{}-{}-{}x{}{}",
                            shot.name,
                            scenario.map_or("legacy", Scenario::id),
                            shot.theme,
                            size.width,
                            size.height,
                            if options.ascii { "-ascii" } else { "" }
                        )
                    } else {
                        shot.name.to_string()
                    };
                    // Duplicate selection flags must not silently inflate totals.
                    if !written.insert(name.clone()) {
                        continue;
                    }
                    let buffer = render(&shot, &snapshot, *size, options.ascii, scenario, now);
                    let provenance = format!(
                        "{source}; shot={}; scenario={}; theme={}; size={}x{}; ascii={}",
                        shot.name,
                        scenario.map_or("legacy", Scenario::id),
                        shot.theme,
                        size.width,
                        size.height,
                        options.ascii
                    );
                    match options.text {
                        Some(TextMode::Grid) => println!(
                            "\n=== {name} ===\nSource: {provenance}\n{}",
                            to_text(&buffer)
                        ),
                        Some(TextMode::Summary) => println!(
                            "\n=== {name} ===\nSource: {provenance}\n{}",
                            summary(&snapshot, scenario, now)
                        ),
                        None => {
                            let mut svg = to_svg(&buffer, theme::resolve(shot.theme));
                            if preview {
                                let end = svg.find('>').expect("SVG opening tag");
                                svg.insert_str(
                                    end + 1,
                                    &format!(
                                        "\n<title>{}</title><desc>{}</desc>",
                                        escape(&name),
                                        escape(&provenance)
                                    ),
                                );
                            }
                            let path = out_dir.join(format!("{name}.svg"));
                            std::fs::write(&path, svg)?;
                            if options.cells {
                                std::fs::write(
                                    out_dir.join(format!("{name}.cells.json")),
                                    cells_json(&buffer, &provenance),
                                )?;
                            }
                            println!("wrote {}", path.display());
                        }
                    }
                }
            }
        }
    }
    eprintln!("rendered {} unique frames", written.len());
    Ok(())
}

fn render(
    shot: &Shot,
    snapshot: &Snapshot,
    size: Size,
    ascii: bool,
    scenario: Option<Scenario>,
    now: Instant,
) -> Buffer {
    let mut config = Config::default();
    config.ui.theme = shot.theme.to_string();
    if ascii {
        config.ui.unicode = false;
        config.ui.show_icons = false;
    }
    let mut app = App::new(config);
    app.snapshot = snapshot.clone();
    app.view = shot.view;
    app.paused = scenario == Some(Scenario::Paused);

    // The text overlays are opened through the same actions a key would use,
    // so their state is real rather than an empty struct that happens to draw.
    let at = now;
    match shot.overlay {
        Overlay::Palette => {
            app.handle(Action::OpenPalette, at);
        }
        Overlay::Command => {
            app.handle(Action::OpenCommandLine, at);
        }
        other => app.overlay = other,
    }
    for c in shot.typed.chars() {
        app.handle(Action::Input(c), at);
    }
    app.pending = shot.pending;
    // A fixed frame number keeps the blinking LIVE badge lit, so regenerating
    // the screenshots produces byte-identical output.
    app.frame = 3;
    app.version = SHOWN_VERSION.to_string();

    let alert = if scenario == Some(Scenario::Drops) {
        Some(AlertKind::NetworkDropped)
    } else if scenario.is_none() || scenario == Some(Scenario::Healthy) {
        shot.alert
    } else {
        None
    };
    if let Some(kind) = alert {
        // A fixed instant far in the future keeps the banner up without
        // depending on when the generator happens to run.
        let at = now;
        app.banner = Some(obs_stats::tui::app::Banner {
            alert: Alert {
                kind,
                frames: 14,
                window: Duration::from_secs(10),
                at,
            },
            until: at + Duration::from_secs(3600),
        });
        app.alert_count = 2;
    }

    let mut terminal =
        Terminal::new(TestBackend::new(size.width, size.height)).expect("test backend");
    terminal
        .draw(|frame| ui::draw_at(frame, &mut app, now))
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// Ask a live OBS for a snapshot, if one answers quickly.
fn live_snapshot() -> Option<Snapshot> {
    let config = Config::default();
    let params = obs_stats::obs::client::ConnectionParams {
        url: config.connection.url(),
        password: obs_stats::config::resolve_password(&config.connection.password_env).ok()?,
        connect_timeout: Duration::from_millis(1500),
        poll_interval: Duration::from_millis(250),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .ok()?;
    runtime
        .block_on(obs_stats::probe::collect(params, Duration::from_secs(5)))
        .ok()
}

/// Give a live-but-idle OBS a plausible on-air state.
///
/// Everything synthesised here is presentation, not measurement: the CPU,
/// frame times, scenes and inputs stay exactly as OBS reported them.
fn dress_for_camera(snapshot: &mut Snapshot) {
    if !snapshot.stream.active {
        snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 4_215_000,
            timecode: "01:10:15.000".into(),
            congestion: 0.04,
            bytes: 3_100_000_000,
            skipped_frames: 3,
            total_frames: 126_450,
            ..StreamStatus::default()
        };
    }
    if !snapshot.record.active {
        snapshot.record = RecordStatus {
            active: true,
            duration_ms: 4_215_000,
            timecode: "01:10:15.000".into(),
            bytes: 5_400_000_000,
            ..RecordStatus::default()
        };
    }
    snapshot.stream_bitrate = 5_900_000.0;
    snapshot.record_bitrate = 10_200_000.0;
    fill_history(snapshot);
    add_log_lines(snapshot);
    animate_meters(snapshot);
}

/// A complete, plausible snapshot for when no OBS is available.
fn demo_snapshot(now: Instant) -> Snapshot {
    let mut snapshot = Snapshot::new(256);
    snapshot.apply(
        Update::Connected(Box::new(Version {
            obs_version: "31.0.2".into(),
            websocket_version: "5.5.4".into(),
            platform: "linux".into(),
        })),
        Instant::now(),
    );

    snapshot.stats = Stats {
        cpu_usage: 18.4,
        memory_usage: 2_140.0,
        available_disk_space: 412_000.0,
        active_fps: 59.94,
        average_frame_render_time: 4.8,
        render_skipped_frames: 61,
        render_total_frames: 1_204_500,
        output_skipped_frames: 18,
        output_total_frames: 253_100,
        websocket_incoming_messages: 1_284,
        websocket_outgoing_messages: 1_291,
    };
    snapshot.video = VideoSettings {
        base_width: 2560,
        base_height: 1440,
        output_width: 1920,
        output_height: 1080,
        fps_numerator: 60,
        fps_denominator: 1,
    };
    snapshot.scenes = SceneList {
        current_program: "Gameplay".into(),
        current_preview: "Just Chatting".into(),
        scenes: vec![
            Scene {
                name: "Starting Soon".into(),
                index: 4,
            },
            Scene {
                name: "Just Chatting".into(),
                index: 3,
            },
            Scene {
                name: "Gameplay".into(),
                index: 2,
            },
            Scene {
                name: "BRB".into(),
                index: 1,
            },
            Scene {
                name: "Stream Ending".into(),
                index: 0,
            },
        ],
    };
    snapshot.profiles = vec!["Twitch 1080p60".into(), "YouTube 1440p".into()];
    snapshot.current_profile = "Twitch 1080p60".into();
    snapshot.collections = vec!["Main Rig".into(), "Podcast".into()];
    snapshot.current_collection = "Main Rig".into();
    snapshot.inputs = vec![
        audio_input("Mic/Aux", "pulse_input_capture", false, -3.5),
        audio_input("Desktop Audio", "pulse_output_capture", false, -12.0),
        audio_input("Game Capture", "pulse_output_capture", false, -6.0),
        audio_input("Music", "ffmpeg_source", true, -20.0),
    ];

    dress_for_camera(&mut snapshot);
    // The populated fixture represents received samples, not a connected
    // socket still waiting for its first poll. Pin freshness to capture time.
    snapshot.polls = 256;
    snapshot.last_poll = Some(now - Duration::from_secs(1));
    for entry in &mut snapshot.log {
        entry.at = Duration::ZERO;
    }
    snapshot
}

/// Build coherent samples through the real model: rates, baselines, histories,
/// and health are derived exactly as they are for OBS polls, not painted on.
fn scenario_snapshot(scenario: Scenario, now: Instant) -> Snapshot {
    use obs_stats::metrics::{Histories, LogEntry};
    use obs_stats::obs::client::PollData;

    if scenario == Scenario::Loading {
        return Snapshot::new(256);
    }
    let mut snapshot = demo_snapshot(now);
    snapshot.history = Histories::new(256);
    snapshot.polls = 0;
    snapshot.log.clear();
    for step in 0..256_u64 {
        let wobble = wave(step as usize, 17);
        let strained = matches!(scenario, Scenario::Strained | Scenario::Drops);
        let drops = scenario == Scenario::Drops;
        let stats = Stats {
            cpu_usage: if strained {
                78.0 + wobble * 10.0
            } else {
                18.4 + wobble * 5.0
            },
            memory_usage: 2_140.0 + wobble * 80.0,
            available_disk_space: if drops { 4_100.0 } else { 412_000.0 },
            active_fps: if strained { 52.0 + wobble * 2.0 } else { 59.94 },
            average_frame_render_time: if strained {
                11.0 + wobble * 2.0
            } else {
                4.8 + wobble
            },
            render_skipped_frames: 61 + if strained { step / 2 } else { step / 64 },
            render_total_frames: 1_204_500 + step * 60,
            output_skipped_frames: 18 + if drops { step * 2 } else { step / 128 },
            output_total_frames: 253_100 + step * 60,
            websocket_incoming_messages: 1_284 + step * 3,
            websocket_outgoing_messages: 1_291 + step * 3,
        };
        let stream = StreamStatus {
            active: true,
            duration_ms: 3_960_000 + step * 1_000,
            timecode: "01:10:15.000".into(),
            congestion: if drops {
                0.81
            } else if strained {
                0.42
            } else {
                0.04
            },
            bytes: 3_100_000_000 + step * 737_500,
            skipped_frames: 3 + if drops { step * 8 } else { 0 },
            total_frames: 126_450 + step * 60,
            ..StreamStatus::default()
        };
        let record = RecordStatus {
            active: true,
            paused: scenario == Scenario::RecordingPaused,
            duration_ms: 3_960_000 + step * 1_000,
            timecode: "01:10:15.000".into(),
            bytes: 5_400_000_000
                + if scenario == Scenario::RecordingPaused {
                    0
                } else {
                    step * 1_275_000
                },
        };
        snapshot.apply(
            Update::Poll(Box::new(PollData {
                stats,
                stream,
                record,
            })),
            now - Duration::from_secs(256 - step),
        );
    }
    snapshot.log = vec![
        LogEntry {
            at: Duration::from_secs(0),
            level: LogLevel::Info,
            message: "synthetic fixture connected; not production telemetry".into(),
        },
        LogEntry {
            at: Duration::from_secs(14),
            level: LogLevel::Info,
            message: "stream started".into(),
        },
        LogEntry {
            at: Duration::from_secs(255),
            level: if scenario == Scenario::Drops {
                LogLevel::Error
            } else {
                LogLevel::Info
            },
            message: format!("fixture condition: {}", scenario.id()),
        },
    ];
    match scenario {
        Scenario::Offline | Scenario::Retrying => {
            snapshot.last_poll = Some(now - Duration::from_secs(45));
            snapshot.apply(
                Update::Disconnected {
                    reason: "OBS websocket connection closed (synthetic fixture)".into(),
                    hint: Some("Check OBS websocket settings and connectivity"),
                    retry_in: Duration::from_secs(4),
                },
                now,
            );
            // Set a fixture-relative timestamp, independent of Snapshot::started.
            if let Some(entry) = snapshot.log.last_mut() {
                entry.at = Duration::from_secs(300);
            }
            if scenario == Scenario::Retrying {
                snapshot.apply(Update::Connecting { attempt: 3 }, now);
            }
        }
        Scenario::Stale => snapshot.last_poll = Some(now - Duration::from_secs(60)),
        Scenario::Paused => snapshot.last_poll = Some(now - Duration::from_secs(30)),
        _ => {}
    }
    snapshot
}

/// Semantic, color-free output complements the cell grid; unavailable or
/// retained values are explicitly qualified instead of implying live health.
fn summary(snapshot: &Snapshot, scenario: Option<Scenario>, now: Instant) -> String {
    let mut text = format!("Connection: {}\n", snapshot.connection.label());
    if matches!(scenario, Some(Scenario::Paused | Scenario::Stale)) {
        text.push_str("Telemetry: retained observation, not a fresh live reading\n");
    }
    if !snapshot.connection.is_connected() || snapshot.last_poll.is_none() {
        text.push_str(if snapshot.last_poll.is_some() {
            "Telemetry: retained last observation, not live\n"
        } else {
            "Telemetry: no poll received\n"
        });
    } else {
        let age = snapshot.age(now).expect("last poll exists").as_secs();
        let _ = writeln!(
            text,
            "Telemetry age: {age} seconds; paused: {}",
            scenario == Some(Scenario::Paused)
        );
    }
    if snapshot.last_poll.is_none() && scenario == Some(Scenario::Loading) {
        return text;
    }
    let _ = writeln!(
        text,
        "Observed health: {}\nCPU: {:.1} percent; memory: {:.0} MiB\nFPS: {:.2}; target: {:.2}; render time: {:.2} ms\nStream active (last observed): {}; reconnecting: {}; rate: {:.2} Mbps\nNetwork drops: {} of {} ({:.2} percent); congestion: {:.1} percent\nRender skips (model scope): {}; encoder skips (model scope): {}\nRecording active (last observed): {}; paused: {}; write rate: {:.2} Mbps\nScene: {}; audio inputs: {}",
        snapshot.health().label(),
        snapshot.stats.cpu_usage,
        snapshot.stats.memory_usage,
        snapshot.stats.active_fps,
        snapshot.video.target_fps(),
        snapshot.stats.average_frame_render_time,
        snapshot.stream.active,
        snapshot.stream.reconnecting,
        snapshot.stream_bitrate / 1_000_000.0,
        snapshot.stream.skipped_frames,
        snapshot.stream.total_frames,
        snapshot.stream.drop_percent(),
        snapshot.stream.congestion_percent(),
        snapshot.session_render_skipped(),
        snapshot.session_output_skipped(),
        snapshot.record.active,
        snapshot.record.paused,
        snapshot.record_bitrate / 1_000_000.0,
        snapshot.scenes.current_program,
        snapshot.inputs.len()
    );
    text
}

/// Reset is preserved, not converted to a theme color: these are the actual
/// TestBackend cells. SVG itself retains the historical themed Reset mapping.
fn cells_json(buffer: &Buffer, provenance: &str) -> String {
    let cells: Vec<_> = (0..buffer.area.height)
        .flat_map(|y| {
            (0..buffer.area.width).map(move |x| {
                let cell = &buffer[(x, y)];
                serde_json::json!({ "x": x, "y": y, "symbol": cell.symbol(),
                "fg": format!("{:?}", cell.fg), "bg": format!("{:?}", cell.bg),
                "modifiers": format!("{:?}", cell.modifier) })
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "schema": 1, "provenance": provenance,
        "width": buffer.area.width, "height": buffer.area.height,
        "svg_reset_policy": "theme foreground/background; mono sample #E6E6E6/#12120F",
        "cells": cells,
    }))
    .expect("cell JSON serializes")
}

fn audio_input(name: &str, kind: &str, muted: bool, volume_db: f64) -> AudioInput {
    AudioInput {
        name: name.to_string(),
        kind: kind.to_string(),
        muted,
        volume_db,
        volume_mul: 10_f64.powf(volume_db / 20.0),
        peaks: Vec::new(),
    }
}

/// Fill the charts with a deterministic, natural-looking series.
///
/// Uses a fixed integer hash rather than a random source so the screenshots
/// are reproducible across runs.
fn fill_history(snapshot: &mut Snapshot) {
    let cpu = snapshot.stats.cpu_usage.max(12.0);
    let fps = snapshot.stats.active_fps.max(59.0);
    let frame = snapshot.stats.average_frame_render_time.max(3.0);

    for step in 0..snapshot.history.cpu.capacity() {
        let t = step as f64;
        let wobble = wave(step, 7) * 0.5 + wave(step, 13) * 0.3 + wave(step, 29) * 0.2;

        snapshot
            .history
            .cpu
            .push((cpu + wobble * 9.0 - 3.0).max(1.0));
        snapshot
            .history
            .memory
            .push(snapshot.stats.memory_usage + wobble * 60.0);
        snapshot
            .history
            .fps
            .push((fps - wobble.max(0.0) * 1.4).max(48.0));
        snapshot
            .history
            .frame_time
            .push((frame + wobble * 2.2).max(0.5));
        // A slow ramp with encoder wobble reads as a real VBR stream.
        snapshot
            .history
            .stream_bitrate
            .push(5_400_000.0 + (t / 12.0).sin() * 700_000.0 + wobble * 500_000.0);
        snapshot
            .history
            .record_bitrate
            .push(10_000_000.0 + wobble * 900_000.0);
        snapshot
            .history
            .congestion
            .push((wobble * 9.0 - 2.0).max(0.0));
    }
}

/// Deterministic pseudo-noise in `0.0..=1.0`.
fn wave(step: usize, period: usize) -> f64 {
    let phase = (step % period) as f64 / period as f64;
    // Triangle wave, smoothed — cheap and free of any RNG dependency.
    let triangle = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    triangle * triangle * (3.0 - 2.0 * triangle)
}

fn add_log_lines(snapshot: &mut Snapshot) {
    if !snapshot.log.is_empty() {
        return;
    }
    let base = Instant::now();
    let lines: &[(u64, LogLevel, &str)] = &[
        (2, LogLevel::Info, "connected to OBS"),
        (9, LogLevel::Info, "collection → Main Rig"),
        (11, LogLevel::Info, "profile → Twitch 1080p60"),
        (14, LogLevel::Info, "stream started"),
        (18, LogLevel::Info, "record started"),
        (46, LogLevel::Info, "scene → Just Chatting"),
        (95, LogLevel::Info, "Mic/Aux unmuted"),
        (132, LogLevel::Info, "scene → Gameplay"),
        (208, LogLevel::Warn, "stream reconnecting"),
        (214, LogLevel::Info, "stream started"),
        (275, LogLevel::Info, "Music muted"),
        (301, LogLevel::Info, "scene → BRB"),
        (340, LogLevel::Info, "scene → Gameplay"),
        (388, LogLevel::Warn, "input removed: Webcam (old)"),
        (402, LogLevel::Info, "input added: Webcam"),
    ];
    for (at, level, message) in lines {
        snapshot.apply(
            Update::Event {
                level: *level,
                message: (*message).to_string(),
            },
            base + Duration::from_secs(*at),
        );
    }
}

/// Put believable, deterministic levels on the meters.
fn animate_meters(snapshot: &mut Snapshot) {
    let levels: [f32; 4] = [0.62, 0.24, 0.41, 0.0];
    for (index, input) in snapshot.inputs.iter_mut().enumerate() {
        if input.muted {
            input.peaks = Vec::new();
        } else {
            let level = levels[index % levels.len()];
            input.peaks = vec![level, (level * 0.86).max(0.02)];
        }
    }
}

/// Render the buffer as plain text, for eyeballing a layout in a terminal.
fn to_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serialise the cell buffer to a standalone SVG.
///
/// Runs of cells sharing a style are merged into a single `<text>` element,
/// which cuts the file size by roughly an order of magnitude versus one
/// element per cell.
fn to_svg(buffer: &Buffer, theme: &Theme) -> String {
    let grid_width = f32::from(buffer.area.width) * CELL_WIDTH;
    let grid_height = f32::from(buffer.area.height) * CELL_HEIGHT;
    let width = grid_width + PADDING * 2.0;
    let height = grid_height + PADDING * 2.0;
    let page_bg = hex(theme.bg, theme, true);

    let mut svg = String::with_capacity(64 * 1024);
    let _ = write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.0} {height:.0}" width="{width:.0}" height="{height:.0}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', monospace" font-size="{FONT_SIZE}">
<rect width="100%" height="100%" rx="10" fill="{page_bg}"/>
<g transform="translate({PADDING} {PADDING})">
"##
    );

    // Background rectangles first, so glyphs always paint on top.
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            let bg = buffer[(x, y)].bg;
            let mut run = 1;
            while x + run < buffer.area.width && buffer[(x + run, y)].bg == bg {
                run += 1;
            }
            if bg != Color::Reset && hex(bg, theme, true) != page_bg {
                let _ = write!(
                    svg,
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"/>"#,
                    f32::from(x) * CELL_WIDTH,
                    f32::from(y) * CELL_HEIGHT,
                    run as f32 * CELL_WIDTH,
                    CELL_HEIGHT,
                    hex(bg, theme, true)
                );
            }
            x += run;
        }
    }

    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            let cell = &buffer[(x, y)];
            let style = (cell.fg, cell.modifier);

            let mut run = 1;
            while x + run < buffer.area.width {
                let next = &buffer[(x + run, y)];
                if (next.fg, next.modifier) != style {
                    break;
                }
                run += 1;
            }

            let text: String = (0..run)
                .map(|offset| buffer[(x + offset, y)].symbol())
                .collect();

            if !text.trim().is_empty() {
                let mut attrs = String::new();
                if cell.modifier.contains(Modifier::BOLD) {
                    attrs.push_str(r#" font-weight="700""#);
                }
                if cell.modifier.contains(Modifier::ITALIC) {
                    attrs.push_str(r#" font-style="italic""#);
                }
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    attrs.push_str(r#" text-decoration="underline""#);
                }
                if cell.modifier.contains(Modifier::DIM) {
                    attrs.push_str(r#" opacity="0.65""#);
                }
                // textLength pins the run to exactly `run` cells wide. Without
                // it the rendered advance depends on whichever monospace font
                // the viewer has, and long lines drift off the right edge.
                let _ = write!(
                    svg,
                    r#"<text x="{:.1}" y="{:.1}" textLength="{:.1}" lengthAdjust="spacingAndGlyphs" fill="{}" xml:space="preserve"{}>{}</text>"#,
                    f32::from(x) * CELL_WIDTH,
                    f32::from(y) * CELL_HEIGHT + BASELINE,
                    run as f32 * CELL_WIDTH,
                    hex(cell.fg, theme, false),
                    attrs,
                    escape(&text)
                );
            }
            x += run;
        }
    }

    svg.push_str("</g></svg>\n");
    svg
}

/// Map a ratatui colour to an SVG hex string.
///
/// `Color::Reset` has no value of its own, so it resolves to the theme's own
/// background or foreground depending on which slot it occupies.
fn hex(color: Color, theme: &Theme, is_background: bool) -> String {
    let rgb = |r: u8, g: u8, b: u8| format!("#{r:02X}{g:02X}{b:02X}");
    match color {
        Color::Rgb(r, g, b) => rgb(r, g, b),
        Color::Reset => {
            if is_background {
                // The mono theme leaves the background to the terminal; pick a
                // neutral dark so the SVG is still legible standalone.
                match theme.bg {
                    Color::Rgb(r, g, b) => rgb(r, g, b),
                    _ => "#12120F".to_string(),
                }
            } else {
                match theme.fg {
                    Color::Rgb(r, g, b) => rgb(r, g, b),
                    _ => "#E6E6E6".to_string(),
                }
            }
        }
        Color::Black => rgb(0x1C, 0x1C, 0x1C),
        Color::Red => rgb(0xE0, 0x50, 0x50),
        Color::Green => rgb(0x5C, 0xC9, 0x7A),
        Color::Yellow => rgb(0xE0, 0xB4, 0x4C),
        Color::Blue => rgb(0x58, 0xA6, 0xFF),
        Color::Magenta => rgb(0xBD, 0x93, 0xF9),
        Color::Cyan => rgb(0x56, 0xB6, 0xC2),
        Color::Gray => rgb(0xB0, 0xB0, 0xB0),
        Color::DarkGray => rgb(0x6A, 0x6A, 0x6A),
        Color::LightRed => rgb(0xFF, 0x7B, 0x72),
        Color::LightGreen => rgb(0x7E, 0xE7, 0x93),
        Color::LightYellow => rgb(0xF1, 0xFA, 0x8C),
        Color::LightBlue => rgb(0x79, 0xC0, 0xFF),
        Color::LightMagenta => rgb(0xD2, 0xA8, 0xFF),
        Color::LightCyan => rgb(0x8B, 0xE9, 0xFD),
        Color::White => rgb(0xF0, 0xF0, 0xF0),
        Color::Indexed(index) => {
            // Approximate the xterm cube well enough for a screenshot.
            let value = 0x30 + (u16::from(index) * 3 % 0xB0) as u8;
            rgb(value, value, value)
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use obs_stats::metrics::{ConnectionState, Health};

    fn now() -> Instant {
        Instant::now() + Duration::from_secs(600)
    }

    #[test]
    fn options_preserve_legacy_defaults_and_validate_selections() {
        use clap::CommandFactory;
        Options::command().debug_assert();
        let options = Options::try_parse_from(["screenshot", "--demo"]).unwrap();
        assert!(!options.preview());
        assert_eq!(options.shots().len(), SHOTS.len());
        assert_eq!(
            Options::try_parse_from(["screenshot", "--text"])
                .unwrap()
                .text,
            Some(TextMode::Grid)
        );
        for args in [
            ["--size", "0x24"],
            ["--size", "999x999"],
            ["--size", "80"],
            ["--scenario", "invented"],
            ["--view", "invented"],
            ["--theme", "invented"],
            ["--shot", "invented"],
        ] {
            assert!(Options::try_parse_from(["screenshot", args[0], args[1]]).is_err());
        }
        let options = Options::try_parse_from([
            "screenshot",
            "--view",
            "audio",
            "--theme",
            "mono",
            "--size",
            "80x24",
        ])
        .unwrap();
        assert!(options.preview());
        assert_eq!(options.shots()[0].view, View::Audio);
        assert_eq!(options.shots().len(), 1);
    }

    #[test]
    fn operating_fixtures_derive_real_model_health_and_rates() {
        let now = now();
        for (scenario, expected) in [
            (Scenario::Healthy, Health::Healthy),
            (Scenario::Strained, Health::Strained),
            (Scenario::Drops, Health::Dropping),
        ] {
            let snapshot = scenario_snapshot(scenario, now);
            assert_eq!(snapshot.health(), expected, "{scenario:?}");
            assert_eq!(snapshot.polls, 256);
            assert_eq!(snapshot.history.cpu.len(), 256);
            assert_eq!(snapshot.stream_bitrate, 5_900_000.0);
            assert_eq!(snapshot.record_bitrate, 10_200_000.0);
            assert_eq!(snapshot.history.cpu.latest(), snapshot.stats.cpu_usage);
        }
        let paused = scenario_snapshot(Scenario::RecordingPaused, now);
        assert!(paused.record.paused);
        assert_eq!(paused.record_bitrate, 0.0);
        let stale = scenario_snapshot(Scenario::Stale, now);
        assert_eq!(stale.age(now), Some(Duration::from_secs(60)));
        assert!(summary(&stale, Some(Scenario::Stale), now).contains("60 seconds"));
    }

    #[test]
    fn unavailability_fixtures_do_not_claim_live_telemetry() {
        let now = now();
        let loading = scenario_snapshot(Scenario::Loading, now);
        assert_eq!(loading.polls, 0);
        assert!(loading.last_poll.is_none());
        assert!(loading.history.cpu.is_empty());
        let text = summary(&loading, Some(Scenario::Loading), now);
        assert!(text.contains("no poll received"));
        assert!(!text.contains("HEALTHY"));
        for scenario in [Scenario::Offline, Scenario::Retrying] {
            let snapshot = scenario_snapshot(scenario, now);
            assert!(!snapshot.connection.is_connected());
            assert_eq!(snapshot.stream_bitrate, 0.0);
            assert!(snapshot.inputs.iter().all(|input| input.peaks.is_empty()));
            assert!(summary(&snapshot, Some(scenario), now).contains("not live"));
        }
        assert!(matches!(
            scenario_snapshot(Scenario::Retrying, now).connection,
            ConnectionState::Connecting { attempt: 3 }
        ));
    }

    #[test]
    fn scenario_size_shot_sweep_is_deterministic_and_ascii_safe() {
        let first = now();
        let later = first + Duration::from_secs(3_600);
        let sizes = ["1x1", "19x7", "60x18", "80x24", "132x38", "180x50"];
        for scenario in Scenario::ALL {
            let a = scenario_snapshot(scenario, first);
            let b = scenario_snapshot(scenario, later);
            for size in sizes {
                let size = size.parse().unwrap();
                for shot in SHOTS {
                    for ascii in [false, true] {
                        let one = render(shot, &a, size, ascii, Some(scenario), first);
                        let two = render(shot, &b, size, ascii, Some(scenario), later);
                        assert_eq!(one, two, "{} {scenario:?} {size:?}", shot.name);
                        if ascii {
                            assert!(
                                !to_text(&one).chars().any(|c| matches!(c as u32,
                                    0x2190..=0x21FF | 0x2500..=0x259F | 0x25A0..=0x25FF | 0x1F000..=0x1FFFF)),
                                "{} {scenario:?} {size:?}",
                                shot.name
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_view_and_theme_remains_deterministic() {
        let first = now();
        let later = first + Duration::from_secs(3_600);
        for theme in theme::THEMES {
            for view in View::ALL {
                let shot = Shot {
                    view,
                    theme: theme.id,
                    ..PLAIN
                };
                let size = "132x38".parse().unwrap();
                let a = render(&shot, &demo_snapshot(first), size, false, None, first);
                let b = render(&shot, &demo_snapshot(later), size, false, None, later);
                assert_eq!(
                    to_svg(&a, theme),
                    to_svg(&b, theme),
                    "{} {}",
                    theme.id,
                    view.id()
                );
            }
        }
    }

    #[test]
    fn legacy_demo_has_received_samples_not_warming_up() {
        let now = now();
        let snapshot = demo_snapshot(now);
        assert_eq!(snapshot.polls, 256);
        assert_eq!(snapshot.age(now), Some(Duration::from_secs(1)));
        let buffer = render(
            &PLAIN,
            &snapshot,
            "132x38".parse().unwrap(),
            false,
            None,
            now,
        );
        assert!(!to_text(&buffer).contains("WARMING UP"));
    }

    #[test]
    fn export_is_unique_self_describing_and_write_errors_propagate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_str().unwrap();
        run(Options::try_parse_from([
            "screenshot",
            "--view",
            "overview",
            "--size",
            "80x24",
            "--size",
            "80x24",
            "--cells",
            "--output",
            path,
        ])
        .unwrap())
        .unwrap();
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
        let svg =
            std::fs::read_to_string(directory.path().join("overview-healthy-grafana-80x24.svg"))
                .unwrap();
        assert!(svg.contains("<desc>synthetic fixture; not production telemetry"));
        let impossible = directory.path().join("not-a-directory");
        std::fs::write(&impossible, "existing file").unwrap();
        let options = Options::try_parse_from([
            "screenshot",
            "--demo",
            "--output",
            impossible.to_str().unwrap(),
        ])
        .unwrap();
        assert!(run(options).is_err());
    }

    #[test]
    fn cells_preserve_reset_and_column_coordinates() {
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 2, 1));
        buffer[(1, 0)].set_symbol("X").set_fg(Color::Red);
        let json: serde_json::Value =
            serde_json::from_str(&cells_json(&buffer, "synthetic fixture")).unwrap();
        assert_eq!(json["cells"].as_array().unwrap().len(), 2);
        assert_eq!(json["cells"][1]["x"], 1);
        assert_eq!(json["cells"][1]["symbol"], "X");
        assert_eq!(json["cells"][1]["bg"], "Reset");
        assert_eq!(json["cells"][1]["fg"], "Red");
    }
}

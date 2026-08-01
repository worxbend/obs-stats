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
use std::path::Path;
use std::time::{Duration, Instant};

use obs_stats::config::Config;
use obs_stats::metrics::Snapshot;
use obs_stats::obs::client::{LogLevel, Update};
use obs_stats::obs::stats::{
    AudioInput, RecordStatus, Scene, SceneList, Stats, StreamStatus, Version, VideoSettings,
};
use obs_stats::tui::app::App;
use obs_stats::tui::keys::Overlay;
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
/// Padding around the terminal grid, so the image reads as a window.
const PADDING: f32 = 12.0;

/// Set by `--ascii`, so the fallback rendering can be inspected the same way.
static ASCII: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Shot {
    name: &'static str,
    view: View,
    overlay: Overlay,
    theme: &'static str,
}

const SHOTS: &[Shot] = &[
    Shot {
        name: "overview",
        view: View::Overview,
        overlay: Overlay::None,
        theme: "claude",
    },
    Shot {
        name: "performance",
        view: View::Performance,
        overlay: Overlay::None,
        theme: "claude",
    },
    Shot {
        name: "stream",
        view: View::Stream,
        overlay: Overlay::None,
        theme: "tokyo-night",
    },
    Shot {
        name: "scenes",
        view: View::Scenes,
        overlay: Overlay::None,
        theme: "nord",
    },
    Shot {
        name: "audio",
        view: View::Audio,
        overlay: Overlay::None,
        theme: "gruvbox",
    },
    Shot {
        name: "log",
        view: View::Log,
        overlay: Overlay::None,
        theme: "catppuccin-mocha",
    },
    Shot {
        name: "themes",
        view: View::Overview,
        overlay: Overlay::Themes,
        theme: "dracula",
    },
    Shot {
        name: "help",
        view: View::Overview,
        overlay: Overlay::Help,
        theme: "claude",
    },
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let demo_only = args.iter().any(|arg| arg == "--demo");
    let text_mode = args.iter().any(|arg| arg == "--text");
    ASCII.store(
        args.iter().any(|arg| arg == "--ascii"),
        std::sync::atomic::Ordering::Relaxed,
    );

    let snapshot = if demo_only {
        demo_snapshot()
    } else {
        match live_snapshot() {
            Some(mut live) => {
                eprintln!("using live OBS data");
                // Live OBS is usually idle; overlay the streaming state so the
                // screenshots show the panels a viewer actually cares about.
                dress_for_camera(&mut live);
                live
            }
            None => {
                eprintln!("no OBS reachable — using synthetic demo data");
                demo_snapshot()
            }
        }
    };

    let out_dir = Path::new("docs/images");
    if !text_mode && std::fs::create_dir_all(out_dir).is_err() {
        eprintln!("could not create {}", out_dir.display());
        std::process::exit(1);
    }

    for shot in SHOTS {
        let buffer = render(shot, &snapshot);

        if text_mode {
            println!("\n═══ {} ═══\n{}", shot.name, to_text(&buffer));
            continue;
        }

        let path = out_dir.join(format!("{}.svg", shot.name));
        let svg = to_svg(&buffer, theme::resolve(shot.theme));
        match std::fs::write(&path, svg) {
            Ok(()) => println!("wrote {}", path.display()),
            Err(error) => eprintln!("could not write {}: {error}", path.display()),
        }
    }
}

fn render(shot: &Shot, snapshot: &Snapshot) -> Buffer {
    let mut config = Config::default();
    config.ui.theme = shot.theme.to_string();
    if ASCII.load(std::sync::atomic::Ordering::Relaxed) {
        config.ui.unicode = false;
        config.ui.show_icons = false;
    }
    let mut app = App::new(config);
    app.snapshot = snapshot.clone();
    app.view = shot.view;
    app.overlay = shot.overlay;
    // A fixed frame number keeps the blinking LIVE badge lit, so regenerating
    // the screenshots produces byte-identical output.
    app.frame = 3;

    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test backend");
    terminal
        .draw(|frame| ui::draw(frame, &mut app))
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
fn demo_snapshot() -> Snapshot {
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
    snapshot
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

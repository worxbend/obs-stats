//! Presentation acceptance contracts driven through the real acquisition model.
//! Fixtures are synthetic; this suite never connects to or controls OBS.
use std::time::{Duration, Instant};

use obs_stats::config::Config;
use obs_stats::obs::client::{Inventory, PollData, Update};
use obs_stats::obs::stats::{
    AudioInput, RecordStatus, Scene, SceneList, Stats, StreamStatus, VideoSettings,
};
use obs_stats::tui::app::App;
use obs_stats::tui::keys::Action;
use obs_stats::tui::{theme, ui, view::View};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn fixture(theme: &str, unicode: bool, now: Instant) -> App {
    let mut config = Config::default();
    config.ui.theme = theme.into();
    config.ui.unicode = unicode;
    config.ui.show_icons = unicode;
    let mut app = App::new(config);
    app.apply(Update::Connected(Box::default()), now);
    app.apply(
        Update::Inventory(Box::new(Inventory {
            video: VideoSettings {
                base_width: 1920,
                base_height: 1080,
                output_width: 1920,
                output_height: 1080,
                fps_numerator: 60,
                fps_denominator: 1,
            },
            scenes: SceneList {
                current_program: "Program".into(),
                scenes: vec![Scene {
                    name: "Program".into(),
                    index: 0,
                }],
                ..SceneList::default()
            },
            inputs: vec![AudioInput {
                name: "Microphone".into(),
                kind: "pulse_input_capture".into(),
                peaks: vec![0.25, 0.125],
                ..AudioInput::default()
            }],
            ..Inventory::default()
        })),
        now,
    );
    for step in 0..12 {
        app.apply(
            Update::Poll(Box::new(PollData {
                stats: Stats {
                    cpu_usage: 12.5,
                    memory_usage: 2048.0,
                    available_disk_space: 120_000.0,
                    active_fps: 60.0,
                    average_frame_render_time: 2.0,
                    render_total_frames: 60 * (step + 1),
                    output_total_frames: 60 * (step + 1),
                    ..Stats::default()
                },
                stream: StreamStatus {
                    active: true,
                    bytes: 750_000 * step,
                    duration_ms: 1000 * step,
                    total_frames: 60 * step,
                    ..StreamStatus::default()
                },
                record: RecordStatus {
                    active: true,
                    bytes: 1_000_000 * step,
                    duration_ms: 1000 * step,
                    ..RecordStatus::default()
                },
            })),
            now - Duration::from_secs(11 - step),
        );
    }
    app.frame = 3;
    app
}

fn capture(app: &mut App, width: u16, height: u16, now: Instant) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw_at(frame, app, now)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn every_theme_view_and_retention_state_preserves_background_and_ascii_contracts() {
    let now = Instant::now() + Duration::from_secs(120);
    for palette in theme::THEMES {
        for view in View::ALL {
            for unicode in [false, true] {
                for state in ["fresh", "silent", "paused", "disconnected"] {
                    for (width, height) in [(80, 24), (132, 38)] {
                        let mut app = fixture(palette.id, unicode, now);
                        app.view = view;
                        let at = match state {
                            "silent" => now + Duration::from_secs(60),
                            "paused" => {
                                app.handle(Action::TogglePause, now);
                                now + Duration::from_secs(60)
                            }
                            "disconnected" => {
                                app.apply(
                                    Update::Disconnected {
                                        reason: "synthetic disconnect".into(),
                                        hint: None,
                                        retry_in: Duration::from_secs(2),
                                    },
                                    now,
                                );
                                now
                            }
                            _ => now,
                        };
                        let buffer = capture(&mut app, width, height, at);
                        let screen = text(&buffer);
                        let context = format!(
                            "{}/{view:?}/{state}/{width}x{height}/unicode={unicode}",
                            palette.id
                        );
                        for cell in &buffer.content {
                            assert!(
                                palette.is_background(cell.bg),
                                "{context}: undeclared background {:?}",
                                cell.bg
                            );
                        }
                        if !unicode {
                            assert!(
                                !screen.chars().any(|c| matches!(c as u32,
                                0x2190..=0x21FF | 0x2500..=0x259F | 0x25A0..=0x25FF
                                | 0x2800..=0x28FF | 0x1F000..=0x1FFFF)),
                                "{context}: unsupported drawing glyph"
                            );
                        }
                        if state == "silent" || state == "disconnected" {
                            assert!(
                                screen.contains("STALE"),
                                "{context}: retained values look current"
                            );
                            assert!(
                                !screen.contains(" LIVE "),
                                "{context}: retained output claims LIVE"
                            );
                            assert!(
                                !screen.contains(" HEALTHY "),
                                "{context}: retained output claims HEALTHY"
                            );
                        }
                        if state == "paused" {
                            assert!(
                                screen.contains("PAUSED"),
                                "{context}: display freeze is hidden"
                            );
                            assert!(
                                !screen.contains(" LIVE "),
                                "{context}: frozen output claims LIVE"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn short_compact_overview_keeps_all_four_values_before_decorative_history() {
    let now = Instant::now() + Duration::from_secs(120);
    let mut app = fixture("grafana", true, now);
    let screen = text(&capture(&mut app, 60, 18, now));
    for value in ["6.00 Mbps", "60.0 fps", "12.5%", "0.0%"] {
        assert!(
            screen.contains(value),
            "compact overview hid {value}: {screen}"
        );
    }
}

#[test]
fn render_clock_is_frozen_and_stale_threshold_tracks_the_poll_cadence() {
    let now = Instant::now() + Duration::from_secs(120);
    let mut app = fixture("grafana", true, now);
    app.poll_interval = Duration::from_secs(5);
    let fresh = capture(&mut app, 132, 38, now + Duration::from_secs(15));
    assert!(!text(&fresh).contains("STALE"), "threshold is exclusive");
    let stale = capture(&mut app, 132, 38, now + Duration::from_secs(16));
    assert!(text(&stale).contains("16s ago"));
    let mut later = fixture("grafana", true, now + Duration::from_secs(3600));
    later.poll_interval = Duration::from_secs(5);
    let repeated = capture(&mut later, 132, 38, now + Duration::from_secs(3616));
    assert_eq!(
        stale, repeated,
        "capture used wall-clock time instead of explicit rendering time"
    );
}

#[test]
fn a_handshake_without_stats_does_not_present_measured_zeroes() {
    let now = Instant::now();
    for view in [View::Overview, View::Performance, View::Stream] {
        let mut app = App::new(Config::default());
        app.apply(Update::Connected(Box::default()), now);
        app.view = view;
        let screen = text(&capture(&mut app, 100, 30, now));
        for false_claim in [" HEALTHY ", " LIVE ", "0.00 fps", "0.0 fps", "0 kbps"] {
            assert!(
                !screen.contains(false_claim),
                "{view:?}: no poll but displayed {false_claim}"
            );
        }
        assert!(screen.contains("WARMING UP"));
    }
}

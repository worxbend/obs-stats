//! Frame composition: header, tab strip, body, status bar, overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Tabs};

use crate::tui::app::App;
use crate::tui::keys::Overlay;
use crate::tui::view::View;
use crate::tui::widgets::{header, help, statusbar, themes};
use crate::tui::{views, widgets::chrome};

/// Height of the tab strip.
const TABS_HEIGHT: u16 = 1;

/// Below this height the header and tabs are dropped so the body still has
/// room to say something useful.
const MIN_CHROME_HEIGHT: u16 = 8;

/// Draw one frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Paint the theme background across the whole terminal first; panels draw
    // over it. `Color::Reset` in the mono theme makes this a no-op.
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.bg).fg(app.theme.fg)),
        area,
    );

    if area.height < MIN_CHROME_HEIGHT || area.width < 20 {
        frame.render_widget(
            Paragraph::new(chrome::placeholder("terminal too small", app.theme)),
            area,
        );
        return;
    }

    let [header_area, tabs_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(header::HEIGHT),
        Constraint::Length(TABS_HEIGHT),
        Constraint::Min(4),
        Constraint::Length(statusbar::HEIGHT),
    ])
    .areas(area);

    header::render(frame, header_area, app);
    render_tabs(frame, tabs_area, app);
    views::render(frame, body_area, app);
    statusbar::render(frame, status_area, app);

    match app.overlay {
        Overlay::None => {}
        Overlay::Help => help::render(frame, area, app),
        Overlay::Themes => themes::render(frame, area, app),
    }
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let titles: Vec<Line> = View::ALL
        .iter()
        .enumerate()
        .map(|(index, view)| {
            let label = if app.show_icons {
                format!("{} {}", view.icon(), view.title())
            } else {
                view.title().to_string()
            };
            Line::from(vec![
                Span::styled(format!("{} ", index + 1), Style::default().fg(theme.border)),
                Span::raw(label),
            ])
        })
        .collect();

    let divider = if app.unicode { "│" } else { "|" };
    let tabs = Tabs::new(titles)
        .select(app.view.index())
        .style(Style::default().fg(theme.muted))
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(Span::styled(divider, Style::default().fg(theme.border)));

    frame.render_widget(tabs, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Render one frame at `width` × `height` and return it as plain text.
    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app() -> App {
        App::new(Config::default())
    }

    #[test]
    fn a_normal_terminal_draws_chrome_and_the_active_view() {
        let mut app = app();
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains("obs-stats"));
        assert!(rendered.contains("Overview"));
        assert!(rendered.contains("System"));
        assert!(rendered.contains("Health"));
        assert!(rendered.contains("quit"), "status bar hints");
    }

    #[test]
    fn every_view_renders_without_panicking() {
        for view in View::ALL {
            let mut app = app();
            app.view = view;
            let rendered = render(&mut app, 120, 40);
            assert!(
                rendered.contains(view.title()),
                "{} did not render its tab",
                view.title()
            );
        }
    }

    #[test]
    fn every_view_survives_a_cramped_terminal() {
        // Regression guard: the layouts must clamp rather than underflow when
        // constraints exceed the available rows.
        for view in View::ALL {
            for (width, height) in [(20, 8), (40, 10), (60, 12), (80, 24)] {
                let mut app = app();
                app.view = view;
                let _ = render(&mut app, width, height);
            }
        }
    }

    #[test]
    fn a_terminal_below_the_minimum_says_so_instead_of_drawing_junk() {
        let mut app = app();
        let rendered = render(&mut app, 40, 5);
        assert!(rendered.contains("terminal too small"));

        // Too narrow for the message to fit in full, but it must still be the
        // only thing on screen rather than a clipped dashboard.
        let narrow = render(&mut app, 12, 20);
        assert!(narrow.contains("terminal"), "{narrow}");
        assert!(!narrow.contains("Overview"));
    }

    #[test]
    fn the_help_overlay_draws_over_the_dashboard() {
        let mut app = app();
        app.overlay = Overlay::Help;
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains("Help"));
        assert!(rendered.contains("Next view"));
    }

    #[test]
    fn the_theme_overlay_draws_the_palette_list() {
        let mut app = app();
        app.overlay = Overlay::Themes;
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains("Appearance"));
        assert!(rendered.contains("Nord"));
        assert!(rendered.contains("Enter applies"));
    }

    #[test]
    fn overlays_render_in_a_terminal_too_small_to_hold_them() {
        for overlay in [Overlay::Help, Overlay::Themes] {
            let mut app = app();
            app.overlay = overlay;
            let _ = render(&mut app, 30, 10);
        }
    }

    /// Populate a snapshot enough that charts, meters and lists all draw.
    fn populated(app: &mut App) {
        use crate::obs::stats::{AudioInput, Scene, SceneList, StreamStatus, VideoSettings};

        app.snapshot.version.obs_version = "31.0.2".into();
        app.snapshot.video = VideoSettings {
            base_width: 1920,
            base_height: 1080,
            output_width: 1280,
            output_height: 720,
            fps_numerator: 60,
            fps_denominator: 1,
        };
        app.snapshot.stream = StreamStatus {
            active: true,
            bytes: 1_000_000,
            total_frames: 1000,
            ..StreamStatus::default()
        };
        app.snapshot.scenes = SceneList {
            current_program: "Live".into(),
            scenes: vec![Scene {
                name: "Live".into(),
                index: 0,
            }],
            ..SceneList::default()
        };
        app.snapshot.inputs = vec![AudioInput {
            name: "Mic/Aux".into(),
            kind: "pulse_input_capture".into(),
            volume_db: -6.0,
            peaks: vec![0.4],
            ..AudioInput::default()
        }];
        for step in 0..80 {
            let wobble = f64::from(step % 8);
            app.snapshot.history.cpu.push(20.0 + wobble);
            app.snapshot.history.fps.push(59.0 + wobble / 10.0);
            app.snapshot.history.frame_time.push(3.0 + wobble / 4.0);
            app.snapshot.history.congestion.push(wobble);
            app.snapshot
                .history
                .stream_bitrate
                .push(5_000_000.0 + wobble * 100_000.0);
        }
    }

    #[test]
    fn no_widget_punches_a_hole_in_the_themed_background() {
        // ratatui's Chart paints its plot area via a Canvas whose background
        // defaults to Color::Reset, which wipes the themed background and
        // exposes the terminal's own — wallpaper, transparency and all. Any
        // widget that does the same in future fails here.
        use ratatui::style::Color;

        let mut config = Config::default();
        config.ui.theme = "tokyo-night".to_string();
        let theme = crate::tui::theme::resolve(&config.ui.theme);
        assert!(
            matches!(theme.bg, Color::Rgb(..)),
            "this test needs a theme with a concrete background"
        );

        for view in View::ALL {
            let mut app = App::new(config.clone());
            app.view = view;
            populated(&mut app);

            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer().clone();

            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    let bg = buffer[(x, y)].bg;
                    // A selected row painting its highlight is the one
                    // legitimate departure from the page background.
                    assert!(
                        bg == theme.bg || bg == theme.highlight_bg,
                        "{} leaves cell ({x},{y}) on {bg:?} — the terminal's own background \
                         shows through wherever this is not the theme's",
                        view.id()
                    );
                }
            }
        }
    }

    #[test]
    fn the_mono_theme_deliberately_keeps_the_terminal_background() {
        // The inverse of the test above: mono exists to inherit whatever the
        // terminal already uses, so nothing may force a background on it.
        use ratatui::style::Color;

        let mut config = Config::default();
        config.ui.theme = "mono".to_string();
        let mut app = App::new(config);
        populated(&mut app);

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let bg = buffer[(x, y)].bg;
                assert!(
                    // Selected rows legitimately paint a highlight.
                    bg == Color::Reset || bg == crate::tui::theme::resolve("mono").highlight_bg,
                    "mono forced {bg:?} onto cell ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn every_theme_renders_every_view() {
        // Cheap insurance that no palette contains a value the widgets choke on.
        for theme in crate::tui::theme::THEMES {
            let mut config = Config::default();
            config.ui.theme = theme.id.to_string();
            let mut app = App::new(config);
            for view in View::ALL {
                app.view = view;
                let _ = render(&mut app, 100, 30);
            }
        }
    }

    /// Ranges `--ascii` exists to avoid: the glyphs a minimal console font is
    /// most likely to be missing.
    ///
    /// General Punctuation (`…`, `—`) is deliberately *not* included — those
    /// are near-universally available and substituting them would make the
    /// output worse for no benefit. The docs say exactly this.
    fn is_drawing_glyph(c: char) -> bool {
        matches!(c as u32,
            0x2190..=0x21FF   // Arrows
            | 0x2500..=0x257F // Box Drawing
            | 0x2580..=0x259F // Block Elements
            | 0x25A0..=0x25FF // Geometric Shapes
            | 0x1F000..=0x1FAFF // Emoji planes
        )
    }

    #[test]
    fn ascii_mode_emits_no_drawing_glyphs_in_any_view_or_overlay() {
        for view in View::ALL {
            for overlay in [Overlay::None, Overlay::Help, Overlay::Themes] {
                let mut config = Config::default();
                config.ui.unicode = false;
                config.ui.show_icons = false;
                let mut app = App::new(config);
                app.view = view;
                app.overlay = overlay;
                // Populate enough state that the data paths render too, not
                // just the empty-state placeholders.
                app.snapshot.version.obs_version = "31.0.2".into();
                app.snapshot.version.platform = "linux".into();
                app.snapshot.video = crate::obs::stats::VideoSettings {
                    base_width: 1920,
                    base_height: 1080,
                    output_width: 1280,
                    output_height: 720,
                    fps_numerator: 60,
                    fps_denominator: 1,
                };
                app.snapshot.stream = crate::obs::stats::StreamStatus {
                    active: true,
                    ..crate::obs::stats::StreamStatus::default()
                };
                app.snapshot.inputs = vec![crate::obs::stats::AudioInput {
                    name: "Mic/Aux".into(),
                    kind: "pulse_input_capture".into(),
                    volume_db: -6.0,
                    peaks: vec![0.4, 0.2],
                    ..crate::obs::stats::AudioInput::default()
                }];
                // Populate the histories, or the charts and sparklines draw
                // nothing and this test passes without ever reaching them —
                // which is exactly where the Unicode glyphs actually live.
                for step in 0..64 {
                    let wobble = f64::from(step % 8);
                    app.snapshot.history.cpu.push(20.0 + wobble);
                    app.snapshot.history.fps.push(59.0 + wobble / 10.0);
                    app.snapshot.history.frame_time.push(3.0 + wobble / 4.0);
                    app.snapshot.history.congestion.push(wobble);
                    app.snapshot
                        .history
                        .stream_bitrate
                        .push(5_000_000.0 + wobble * 100_000.0);
                }

                let rendered = render(&mut app, 120, 40);
                if let Some(bad) = rendered.chars().find(|c| is_drawing_glyph(*c)) {
                    panic!(
                        "{}/{overlay:?} rendered {bad:?} (U+{:04X}) in ASCII mode",
                        view.id(),
                        bad as u32
                    );
                }
            }
        }
    }

    #[test]
    fn ascii_mode_avoids_unicode_block_glyphs_in_the_chrome() {
        let mut app = app();
        app.unicode = false;
        app.show_icons = false;
        let rendered = render(&mut app, 120, 40);
        assert!(!rendered.contains('█'));
        assert!(!rendered.contains('╭'), "rounded borders are Unicode-only");
    }

    #[test]
    fn the_tab_strip_numbers_every_view() {
        let mut app = app();
        // Icons sit between the digit and the title; drop them so the
        // assertion is about the numbering, not the glyph.
        app.show_icons = false;
        let rendered = render(&mut app, 120, 40);
        for (index, view) in View::ALL.iter().enumerate() {
            assert!(
                rendered.contains(&format!("{} {}", index + 1, view.title())),
                "tab {} is not numbered",
                view.title()
            );
        }
    }

    #[test]
    fn a_populated_snapshot_reaches_the_screen() {
        use crate::obs::client::{Inventory, PollData, Update};
        use crate::obs::stats::{AudioInput, Scene, SceneList, Stats, StreamStatus};
        use std::time::Instant;

        let mut app = app();
        let now = Instant::now();
        app.apply(
            Update::Inventory(Box::new(Inventory {
                scenes: SceneList {
                    current_program: "Live".into(),
                    scenes: vec![Scene {
                        name: "Live".into(),
                        index: 0,
                    }],
                    ..SceneList::default()
                },
                inputs: vec![AudioInput {
                    name: "Mic/Aux".into(),
                    kind: "pulse_input_capture".into(),
                    ..AudioInput::default()
                }],
                profiles: vec!["Twitch".into()],
                current_profile: "Twitch".into(),
                ..Inventory::default()
            })),
            now,
        );
        app.apply(
            Update::Poll(Box::new(PollData {
                stats: Stats {
                    cpu_usage: 12.5,
                    active_fps: 60.0,
                    ..Stats::default()
                },
                stream: StreamStatus {
                    active: true,
                    ..StreamStatus::default()
                },
                ..PollData::default()
            })),
            now,
        );

        assert!(render(&mut app, 120, 40).contains("LIVE"));

        app.view = View::Scenes;
        let scenes = render(&mut app, 120, 40);
        assert!(scenes.contains("Live"));
        assert!(scenes.contains("PROGRAM"));
        assert!(scenes.contains("Twitch"));

        app.view = View::Audio;
        assert!(render(&mut app, 120, 40).contains("Mic/Aux"));
    }
}

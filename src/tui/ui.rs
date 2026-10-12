//! Frame composition: header, tab strip, body, status bar, overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::app::App;
use crate::tui::keys::Overlay;
use crate::tui::view::View;
use crate::tui::widgets::{
    banner, cmdline, header, help, palette, scrollbar, statusbar, themes, whichkey,
};
use crate::tui::{views, widgets::chrome};

/// Height of the tab strip.
const TABS_HEIGHT: u16 = 1;

/// Below this height the header and tabs are dropped so the body still has
/// room to say something useful.
const MIN_CHROME_HEIGHT: u16 = 8;

/// Draw one frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    draw_at(frame, app, std::time::Instant::now());
}

/// Render with an explicit clock so age and stale-state captures are reproducible.
pub fn draw_at(frame: &mut Frame, app: &mut App, now: std::time::Instant) {
    app.render_at = now;
    let area = frame.area();

    // Anything clickable has to re-register itself this frame. A rect left
    // over from the last one would route a click to a widget that is no
    // longer on screen.
    app.begin_frame();

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

    // The banner only takes a row when there is one to show, so an untroubled
    // dashboard loses nothing to it.
    let banner_height = if app.banner.is_some() || app.telemetry_stale() {
        banner::HEIGHT
    } else {
        0
    };

    let [header_area, tabs_area, banner_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(header::HEIGHT),
        Constraint::Length(TABS_HEIGHT),
        Constraint::Length(banner_height),
        Constraint::Min(4),
        Constraint::Length(statusbar::HEIGHT),
    ])
    .areas(area);

    header::render(frame, header_area, app);
    render_tabs(frame, tabs_area, app);
    app.regions.tabs = tabs_area;
    if app.banner.is_some() {
        banner::render(frame, banner_area, app);
    } else if app.telemetry_stale() {
        let age = app
            .snapshot
            .age(now)
            .map(|age| format!("{}s ago", age.as_secs()))
            .unwrap_or_else(|| "age unavailable".into());
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" STALE / last sample {age} / retained telemetry, not current health"),
                Style::default()
                    .fg(app.theme.warning)
                    .bg(app.theme.surface_hi()),
            ))),
            banner_area,
        );
    }
    views::render(frame, body_area, app);
    app.regions.body = body_area;
    render_scrollbar(frame, app);

    // The command line takes the status row rather than floating, so the two
    // are drawn as alternatives.
    if app.overlay == Overlay::Command {
        cmdline::render(frame, area, status_area, app);
    } else {
        statusbar::render(frame, status_area, app);
    }

    match app.overlay {
        Overlay::None | Overlay::Command => {}
        Overlay::Help => {
            let panel = help::render(frame, area, app);
            app.regions.overlay_panel = Some(panel);
        }
        Overlay::Themes => {
            let mut state = app.take_overlay_list_state();
            let (panel, list) = themes::render(frame, area, app, &mut state);
            app.put_overlay_list_state(panel, list, state);
        }
        Overlay::Palette => {
            let mut state = app.take_overlay_list_state();
            let (panel, list) = palette::render(frame, area, app, &mut state);
            app.put_overlay_list_state(panel, list, state);
        }
    }

    // Which-key sits above everything: it is feedback on the key just pressed,
    // so it must not be buried under whatever that key is about to open.
    whichkey::render(frame, area, app);
}

/// Draw the active view's scrollbar over the right border of its list panel.
///
/// Nothing is drawn when the whole list fits, so a short list is not decorated
/// with a bar that cannot move.
fn render_scrollbar(frame: &mut Frame, app: &mut App) {
    let Some(list) = app.regions.list else {
        return;
    };
    let total = app.row_count(app.view);
    if total <= list.height as usize || list.height == 0 {
        return;
    }

    // The panel's right border, which the bar replaces rather than eating a
    // column of content.
    let track = Rect {
        x: list.x + list.width,
        y: list.y,
        width: 1,
        height: list.height,
    };
    if track.x >= frame.area().right() {
        return;
    }

    scrollbar::render(
        frame,
        track,
        total,
        app.list_offset(app.view),
        app.theme,
        app.unicode,
    );
    app.regions.scrollbar = Some(track);
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    // One leading space, then ` N label ` pills separated by single cells.
    // `mouse::tab_at` walks this exact layout, so the two cannot drift.
    let mut spans = vec![Span::raw(" ")];
    for (index, view) in View::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        if *view == app.view {
            // The tab is the primary navigation mark; reserve solid brand
            // colour for it and keep list selections on the quieter highlight.
            // Span widths stay identical to mouse::tab_at's mirror.
            spans.push(Span::styled(
                format!(" {} {} ", index + 1, view.tab_label(app.show_icons)),
                Style::default()
                    .fg(theme.on_color(theme.accent))
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", index + 1),
                Style::default().fg(theme.border),
            ));
            spans.push(Span::styled(
                format!("{} ", view.tab_label(app.show_icons)),
                Style::default().fg(theme.muted),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.surface())),
        area,
    );
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
    fn silent_connected_telemetry_is_stale_instead_of_healthy_or_live() {
        use crate::obs::client::{PollData, Update};
        use crate::obs::stats::{Stats, StreamStatus};
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut app = app();
        app.apply(Update::Connected(Box::default()), now);
        app.apply(
            Update::Poll(Box::new(PollData {
                stats: Stats {
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
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| draw_at(frame, &mut app, now + Duration::from_secs(60)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("STALE"));
        assert!(screen.contains("60s ago"), "no last-sample age: {screen}");
        assert!(
            !screen.contains(" HEALTHY "),
            "stale measurements claimed current health"
        );
        assert!(!screen.contains(" LIVE "), "stale output claimed live");
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
        for overlay in ALL_OVERLAYS {
            for (width, height) in [(30, 10), (20, 8), (12, 4)] {
                let mut app = opened(overlay);
                let _ = render(&mut app, width, height);
            }
        }
    }

    /// Every modal layer, so the sweeps below cannot miss a new one.
    const ALL_OVERLAYS: [Overlay; 5] = [
        Overlay::None,
        Overlay::Help,
        Overlay::Themes,
        Overlay::Command,
        Overlay::Palette,
    ];

    /// An app with `overlay` opened the way a key would open it, so the
    /// overlay's own state is populated rather than left at its default.
    fn opened(overlay: Overlay) -> App {
        use crate::tui::keys::Action;
        use std::time::Instant;

        let mut app = app();
        let now = Instant::now();
        match overlay {
            Overlay::None => {}
            Overlay::Help => {
                app.handle(Action::ToggleHelp, now);
            }
            Overlay::Themes => {
                app.handle(Action::ToggleThemePicker, now);
            }
            Overlay::Command => {
                app.handle(Action::OpenCommandLine, now);
                for c in "theme no".chars() {
                    app.handle(Action::Input(c), now);
                }
            }
            Overlay::Palette => {
                app.handle(Action::OpenPalette, now);
                for c in "the".chars() {
                    app.handle(Action::Input(c), now);
                }
            }
        }
        assert_eq!(app.overlay, overlay);
        app
    }

    #[test]
    fn the_command_line_takes_the_status_row() {
        let mut app = opened(Overlay::Command);
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains(":theme no"), "{rendered}");
        // The hints it replaces must not also be on screen.
        assert!(!rendered.lines().last().unwrap().contains("q quit"));
    }

    #[test]
    fn the_palette_draws_over_the_dashboard() {
        let mut app = opened(Overlay::Palette);
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("theme"));
    }

    #[test]
    fn a_pending_key_sequence_puts_which_key_on_screen() {
        use crate::tui::keys::Pending;

        let mut app = app();
        let quiet = render(&mut app, 120, 40);
        assert!(!quiet.contains("+UI toggles"));

        app.pending = Pending::Leader;
        let rendered = render(&mut app, 120, 40);
        assert!(rendered.contains("+UI toggles"), "{rendered}");
        assert!(rendered.contains("Quit"));
    }

    #[test]
    fn which_key_draws_above_whatever_else_is_open() {
        use crate::tui::keys::Pending;

        // It is feedback on the key just pressed, so being buried under an
        // overlay would make it useless exactly when it is needed.
        for overlay in ALL_OVERLAYS {
            let mut app = opened(overlay);
            app.pending = Pending::LeaderGroup('u');
            let rendered = render(&mut app, 120, 40);
            assert!(
                rendered.contains("Unicode drawing"),
                "{overlay:?} hid the which-key popup:\n{rendered}"
            );
        }
    }

    #[test]
    fn a_long_list_draws_a_scrollbar_and_a_short_one_does_not() {
        use crate::metrics::LogEntry;
        use crate::obs::client::LogLevel;
        use std::time::Duration;

        let mut app = app();
        app.view = View::Log;
        app.snapshot.log = (0..3)
            .map(|index| LogEntry {
                level: LogLevel::Info,
                message: format!("event {index}"),
                at: Duration::from_secs(index),
            })
            .collect();
        let _ = render(&mut app, 120, 40);
        assert!(app.regions.scrollbar.is_none(), "three rows always fit");

        app.snapshot.log = (0..500)
            .map(|index| LogEntry {
                level: LogLevel::Info,
                message: format!("event {index}"),
                at: Duration::from_secs(index),
            })
            .collect();
        let _ = render(&mut app, 120, 40);
        let track = app.regions.scrollbar.expect("500 rows must scroll");
        let list = app.regions.list.unwrap();
        assert_eq!(track.height, list.height);
        assert_eq!(track.x, list.x + list.width, "the bar sits on the border");
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

        // Every view crossed with every overlay and the which-key popup: each
        // of those draws a `Clear`, which is the exact widget that reintroduces
        // this bug if whatever follows it does not repaint the background.
        for view in View::ALL {
            for overlay in ALL_OVERLAYS {
                for pending in [
                    crate::tui::keys::Pending::None,
                    crate::tui::keys::Pending::Leader,
                ] {
                    let mut app = opened(overlay);
                    app.config = config.clone();
                    app.theme = theme;
                    app.view = view;
                    app.pending = pending;
                    populated(&mut app);
                    with_banner(&mut app, crate::metrics::AlertKind::NetworkDropped);

                    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
                    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                    let buffer = terminal.backend().buffer().clone();

                    for y in 0..buffer.area.height {
                        for x in 0..buffer.area.width {
                            let bg = buffer[(x, y)].bg;
                            // Every painted background must be one of the
                            // theme's declared surface or signal tokens —
                            // the panels' surface fill, the overlays' raised
                            // fill, selection highlights, badges and the
                            // alert banner are all deliberate. Anything else
                            // is the terminal's own background leaking
                            // through where the theme should be.
                            assert!(
                                theme.is_background(bg),
                                "{}/{overlay:?}/{pending:?} leaves cell ({x},{y}) on {bg:?} — \
                                 the terminal's own background shows through wherever this is \
                                 not the theme's",
                                view.id()
                            );
                        }
                    }
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

    /// Put a frame-loss banner on screen the way a real poll would.
    fn with_banner(app: &mut App, kind: crate::metrics::AlertKind) {
        use crate::metrics::alerts::Alert;
        use std::time::Duration;

        let now = std::time::Instant::now();
        app.banner = Some(crate::tui::app::Banner {
            alert: Alert {
                kind,
                frames: 12,
                window: Duration::from_secs(10),
                at: now,
            },
            until: now + Duration::from_secs(30),
        });
        app.alert_count = 1;
    }

    #[test]
    fn the_alert_banner_is_visible_from_every_view() {
        // A dropped frame matters whichever tab you happen to be on.
        for view in View::ALL {
            let mut app = app();
            app.view = view;
            with_banner(&mut app, crate::metrics::AlertKind::NetworkDropped);

            let rendered = render(&mut app, 120, 40);
            assert!(
                rendered.contains("DROPPING FRAMES"),
                "{} hid the banner",
                view.id()
            );
            assert!(
                rendered.contains("viewers are seeing stutter"),
                "{}",
                view.id()
            );
        }
    }

    #[test]
    fn the_banner_costs_no_rows_when_there_is_nothing_to_report() {
        let mut quiet = app();
        let without = render(&mut quiet, 120, 40);

        let mut noisy = app();
        with_banner(&mut noisy, crate::metrics::AlertKind::NetworkDropped);
        let with = render(&mut noisy, 120, 40);

        assert!(!without.contains("DROPPING FRAMES"));
        assert_eq!(
            without.lines().count(),
            with.lines().count(),
            "the frame is a fixed height either way"
        );
    }

    #[test]
    fn the_banner_survives_a_cramped_terminal() {
        for kind in crate::metrics::AlertKind::ALL {
            for (width, height) in [(20, 8), (40, 10), (80, 24)] {
                let mut app = app();
                with_banner(&mut app, kind);
                let _ = render(&mut app, width, height);
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
            for overlay in ALL_OVERLAYS {
                let mut config = Config::default();
                config.ui.unicode = false;
                config.ui.show_icons = false;
                let mut app = App::new(config);
                app.view = view;
                app.overlay = overlay;
                // A pending sequence puts the which-key popup on screen too,
                // so its glyphs are swept along with everything else.
                app.pending = crate::tui::keys::Pending::Leader;
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
                // The banner has its own glyphs, so it belongs in this sweep.
                with_banner(&mut app, crate::metrics::AlertKind::NetworkDropped);
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
        // A poll is data, not proof of a live connection: the real session
        // always reports the successful handshake before inventory and stats.
        app.apply(Update::Connected(Box::default()), now);
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

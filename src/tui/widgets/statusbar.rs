//! The bottom bar: key hints, poll cadence, and transient messages.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::ConnectionState;
use crate::tui::app::{App, Hit};
use crate::tui::keys::Overlay;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Height the status bar occupies.
pub const HEIGHT: u16 = 1;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(28)]).areas(area);

    let hints = hints(app);
    let (state, targets) = state(app);
    let hit_rects = right_aligned_rects(&state, &targets, right);

    frame.render_widget(Paragraph::new(hints), left);
    frame.render_widget(Paragraph::new(state).alignment(Alignment::Right), right);

    app.regions.hits.extend(hit_rects);
}

/// Where each targeted span lands, given that the line is right-aligned in
/// `area`.
///
/// Right alignment means positions can only be known once the whole line is
/// built, which is why the spans and their targets are produced together and
/// measured here rather than at the point each is pushed.
fn right_aligned_rects(
    line: &Line<'static>,
    targets: &[(usize, Hit)],
    area: Rect,
) -> Vec<(Rect, Hit)> {
    let widths: Vec<u16> = line
        .spans
        .iter()
        .map(|span| span.content.chars().count() as u16)
        .collect();
    let total: u16 = widths.iter().sum();

    // A line wider than its column is clipped from the left by the alignment,
    // so there is no honest rect to report for anything in it.
    if total > area.width {
        return Vec::new();
    }
    let mut cursor = area.x + area.width - total;

    let mut rects = Vec::with_capacity(targets.len());
    for (index, width) in widths.iter().enumerate() {
        if let Some((_, hit)) = targets.iter().find(|(target, _)| *target == index) {
            rects.push((
                Rect {
                    x: cursor,
                    y: area.y,
                    width: *width,
                    height: 1,
                },
                *hit,
            ));
        }
        cursor += width;
    }
    rects
}

/// Key hints, or the active toast when there is one.
///
/// A toast replaces the hints rather than sharing the row: on an 80-column
/// terminal there is not room for both, and the message is the thing the user
/// just asked for.
fn hints(app: &App) -> Line<'static> {
    let theme = app.theme;

    if let Some(toast) = &app.toast {
        return Line::from(vec![
            Span::styled(
                "▸ ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(toast.message.clone(), Style::default().fg(theme.accent_alt)),
        ]);
    }

    // A dead connection is more useful than a key reminder, so it wins the row.
    match &app.snapshot.connection {
        ConnectionState::Disconnected { reason, hint, .. } => {
            return problem(reason, *hint, theme);
        }
        ConnectionState::Fatal { reason, hint } => return problem(reason, *hint, theme),
        _ => {}
    }

    // The picker hint names the cursor keys, which have no ASCII glyph.
    let cursor_keys = if app.unicode { "↑↓" } else { "up/dn" };
    let pairs: &[(&str, &str)] = match app.overlay {
        Overlay::Help => &[("Esc", "close")],
        Overlay::Themes => &[
            (cursor_keys, "preview"),
            ("Enter", "apply"),
            ("Esc", "cancel"),
        ],
        Overlay::Palette => &[(cursor_keys, "move"), ("Enter", "run"), ("Esc", "cancel")],
        // The command line draws its own row, so these are only reached if it
        // is somehow open without one.
        Overlay::Command => &[("Tab", "complete"), ("Enter", "run"), ("Esc", "cancel")],
        Overlay::None => &[
            ("space", "keys"),
            ("Tab", "view"),
            ("r", "refresh"),
            ("p", "pause"),
            (":", "command"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };

    let glyphs = chrome::glyphs(app.unicode);
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (index, (key, label)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                format!(" {} ", glyphs.separator),
                Style::default().fg(theme.border),
            ));
        }
        // The key is a keycap chip, the action plain text beside it — the
        // same visual language as the which-key popup.
        spans.extend(chrome::kbd(key, theme));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(theme.muted),
        ));
    }
    Line::from(spans)
}

fn problem(reason: &str, hint: Option<&'static str>, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            "! ",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(reason.to_string(), Style::default().fg(theme.danger)),
    ];
    if let Some(hint) = hint {
        spans.push(Span::styled(
            format!("  {hint}"),
            Style::default().fg(theme.muted),
        ));
    }
    Line::from(spans)
}

/// Right-hand side: paused flag, poll cadence and theme name.
///
/// Returns the spans alongside which of them are click targets, by index. The
/// row is right-aligned, so the caller works out where they actually land.
fn state(app: &App) -> (Line<'static>, Vec<(usize, Hit)>) {
    let theme = app.theme;
    let mut spans = Vec::new();
    let mut targets = Vec::new();

    // The toggle is always present, so there is somewhere to click to pause as
    // well as to resume. `‖` is General Punctuation rather than one of the
    // Geometric Shapes `--ascii` avoids, but the fallback is spelt out anyway.
    targets.push((spans.len(), Hit::Pause));
    if app.paused {
        spans.extend(chrome::badge("PAUSED", theme.warning, theme));
    } else {
        spans.push(Span::styled(
            if app.unicode { "‖" } else { "||" },
            Style::default().fg(theme.border),
        ));
    }
    spans.push(Span::raw(" "));

    targets.push((spans.len(), Hit::Poll));
    spans.push(Span::styled(
        format!("{}ms", app.poll_interval.as_millis()),
        Style::default().fg(theme.muted),
    ));

    spans.push(Span::styled(" · ", Style::default().fg(theme.border)));
    targets.push((spans.len(), Hit::ThemeName));
    spans.push(Span::styled(
        app.theme.label.to_string(),
        Style::default().fg(theme.muted),
    ));

    (Line::from(spans), targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::testing::line_text;
    use std::time::{Duration, Instant};

    fn app() -> App {
        App::new(Config::default())
    }

    #[test]
    fn the_dashboard_shows_its_main_keys() {
        let rendered = line_text(&hints(&app()));
        // `space` opens which-key, which is where the rest of the map lives,
        // and `:` reaches everything by name.
        for key in ["space", "Tab", "r", "p", ":", "?", "q"] {
            assert!(rendered.contains(key), "missing hint for {key}: {rendered}");
        }
    }

    #[test]
    fn hints_change_with_the_overlay() {
        let mut app = app();
        app.overlay = Overlay::Themes;
        let rendered = line_text(&hints(&app));
        assert!(rendered.contains("Enter"));
        assert!(rendered.contains("↑↓"));

        // ASCII mode must name the same keys without arrow glyphs.
        app.unicode = false;
        let ascii = line_text(&hints(&app));
        assert!(ascii.contains("up/dn"));
        assert!(ascii.is_ascii(), "{ascii}");
        app.unicode = true;
        assert!(rendered.contains("cancel"));
        assert!(
            !rendered.contains("refresh"),
            "dashboard keys are inert here"
        );

        app.overlay = Overlay::Help;
        assert!(line_text(&hints(&app)).contains("close"));
    }

    #[test]
    fn a_toast_takes_over_the_hint_row() {
        let mut app = app();
        app.notify("poll interval: 500ms".into(), Instant::now());
        let rendered = line_text(&hints(&app));
        assert!(rendered.contains("poll interval: 500ms"));
        assert!(!rendered.contains("quit"));
    }

    #[test]
    fn a_dropped_connection_outranks_the_key_hints() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Disconnected {
            reason: "could not connect to OBS at ws://127.0.0.1:4455".into(),
            hint: Some("Is OBS running?"),
            retry_in: Duration::from_secs(2),
        };
        let rendered = line_text(&hints(&app));
        assert!(rendered.contains("ws://127.0.0.1:4455"));
        assert!(rendered.contains("Is OBS running?"));
        assert!(!rendered.contains("quit"));
    }

    #[test]
    fn a_fatal_error_is_shown_with_its_hint() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Fatal {
            reason: "authentication failed".into(),
            hint: Some("Check the password"),
        };
        let rendered = line_text(&hints(&app));
        assert!(rendered.contains("authentication failed"));
        assert!(rendered.contains("Check the password"));
    }

    #[test]
    fn a_toast_still_wins_over_a_dropped_connection() {
        // The toast is the direct answer to a key the user just pressed.
        let mut app = app();
        app.snapshot.connection = ConnectionState::Disconnected {
            reason: "closed".into(),
            hint: None,
            retry_in: Duration::from_secs(1),
        };
        app.notify("paused".into(), Instant::now());
        assert!(line_text(&hints(&app)).contains("paused"));
    }

    #[test]
    fn the_right_side_reports_cadence_and_theme() {
        let app = app();
        let (line, _) = state(&app);
        let rendered = line_text(&line);
        assert!(rendered.contains("1000ms"));
        assert!(rendered.contains("Claude"));
        assert!(!rendered.contains("PAUSED"));
    }

    #[test]
    fn pausing_is_flagged_prominently() {
        let mut app = app();
        app.paused = true;
        let (line, _) = state(&app);
        assert!(line_text(&line).contains("PAUSED"));
    }
}

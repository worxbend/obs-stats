//! The bottom bar: key hints, poll cadence, and transient messages.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::ConnectionState;
use crate::tui::app::App;
use crate::tui::keys::Overlay;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Height the status bar occupies.
pub const HEIGHT: u16 = 1;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(28)]).areas(area);

    frame.render_widget(Paragraph::new(hints(app)), left);
    frame.render_widget(
        Paragraph::new(state(app)).alignment(Alignment::Right),
        right,
    );
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

    let pairs: &[(&str, &str)] = match app.overlay {
        Overlay::Help => &[("Esc", "close")],
        Overlay::Themes => &[("↑↓", "preview"), ("Enter", "apply"), ("Esc", "cancel")],
        Overlay::None => &[
            ("Tab", "view"),
            ("1-6", "jump"),
            ("r", "refresh"),
            ("space", "pause"),
            ("t", "theme"),
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
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
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

/// Right-hand side: paused flag and the poll cadence.
fn state(app: &App) -> Line<'static> {
    let theme = app.theme;
    let mut spans = Vec::new();

    if app.paused {
        spans.push(Span::styled(
            "PAUSED ",
            Style::default()
                .fg(theme.highlight_fg)
                .bg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled(
        format!("{}ms", app.poll_interval.as_millis()),
        Style::default().fg(theme.muted),
    ));
    spans.push(Span::styled(
        format!(" · {}", app.theme.label),
        Style::default().fg(theme.muted),
    ));

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::time::{Duration, Instant};

    fn app() -> App {
        App::new(Config::default())
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_dashboard_shows_its_main_keys() {
        let rendered = text(&hints(&app()));
        for key in ["Tab", "1-6", "r", "space", "t", "?", "q"] {
            assert!(rendered.contains(key), "missing hint for {key}: {rendered}");
        }
    }

    #[test]
    fn hints_change_with_the_overlay() {
        let mut app = app();
        app.overlay = Overlay::Themes;
        let rendered = text(&hints(&app));
        assert!(rendered.contains("Enter"));
        assert!(rendered.contains("cancel"));
        assert!(
            !rendered.contains("refresh"),
            "dashboard keys are inert here"
        );

        app.overlay = Overlay::Help;
        assert!(text(&hints(&app)).contains("close"));
    }

    #[test]
    fn a_toast_takes_over_the_hint_row() {
        let mut app = app();
        app.notify("poll interval: 500ms".into(), Instant::now());
        let rendered = text(&hints(&app));
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
        let rendered = text(&hints(&app));
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
        let rendered = text(&hints(&app));
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
        assert!(text(&hints(&app)).contains("paused"));
    }

    #[test]
    fn the_right_side_reports_cadence_and_theme() {
        let app = app();
        let rendered = text(&state(&app));
        assert!(rendered.contains("1000ms"));
        assert!(rendered.contains("Claude"));
        assert!(!rendered.contains("PAUSED"));
    }

    #[test]
    fn pausing_is_flagged_prominently() {
        let mut app = app();
        app.paused = true;
        assert!(text(&state(&app)).contains("PAUSED"));
    }
}

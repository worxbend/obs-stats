//! The Log view — OBS events since this session started.
//!
//! Timestamps are relative to app start rather than wall-clock: the question
//! this view answers is "how long before the drop did the scene change?", and
//! a relative clock answers it without arithmetic.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::metrics::LogEntry;
use crate::obs::client::LogLevel;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::views;
use crate::tui::views::scenes::context_summary;
use crate::tui::widgets::chrome;
use crate::tui::widgets::list;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = app.theme;
    let entries = app.snapshot.log.clone();
    app.set_row_count(View::Log, entries.len());
    // Owned by the app so the settled offset outlives the frame; see
    // `App::take_list_state`.
    let mut state = app.take_list_state(View::Log);

    let [summary, context_area, body] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Min(4),
    ])
    .areas(area);
    render_summary(frame, summary, &entries, app);
    let [list_area, inspection] = if area.width >= 100 {
        Layout::horizontal([Constraint::Min(40), Constraint::Length(36)]).areas(body)
    } else {
        [body, Rect::default()]
    };

    let context_block =
        chrome::panel_with_note("Now", "last reported context", false, theme, app.unicode);
    let context_inner = chrome::frame_panel(frame, context_area, context_block);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "scene / profile / collection   ",
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                context_summary(&app.snapshot),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ])),
        context_inner,
    );

    let block = chrome::panel_with_note(
        "Events",
        format!("{} retained / oldest first", entries.len()),
        true,
        theme,
        app.unicode,
    );
    let panel_inner = chrome::frame_panel(frame, list_area, block);
    let [header, inner] = Layout::vertical([
        Constraint::Length(u16::from(!entries.is_empty())),
        Constraint::Min(0),
    ])
    .areas(panel_inner);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  TIME     LEVEL   EVENT",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ))),
        header,
    );
    if inspection.width > 0 {
        let inner = chrome::frame_panel(
            frame,
            inspection,
            chrome::panel_with_note("Selected event", "inspect", false, theme, app.unicode),
        );
        frame.render_widget(
            Paragraph::new(event_detail(entries.get(app.selection(View::Log)), theme))
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder(
                "no events yet — scene switches and output changes will appear here",
                theme,
            )),
            inner,
        );
        app.put_list_state(View::Log, inner, state);
        return;
    }

    let items: Vec<Line> = entries
        .iter()
        .map(|entry| line(entry, theme, app.unicode))
        .collect();

    list::render_rows(
        frame,
        inner,
        items,
        app.selection(View::Log),
        &mut state,
        theme,
        app.unicode,
    );
    app.put_list_state(View::Log, inner, state);
}

fn render_summary(frame: &mut Frame, area: Rect, entries: &[LogEntry], app: &App) {
    let regions = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(area);
    let warnings = entries
        .iter()
        .filter(|entry| entry.level == LogLevel::Warn)
        .count();
    let errors = entries
        .iter()
        .filter(|entry| entry.level == LogLevel::Error)
        .count();
    let empty = crate::metrics::History::new(1);
    let latest = entries.last().map_or_else(
        || "no events yet".to_string(),
        |entry| {
            format!(
                "latest +{}",
                crate::util::duration_ms(entry.at.as_millis() as u64)
            )
        },
    );
    for (index, title, value, note, color) in [
        (0, "Events", entries.len().to_string(), latest, app.theme.fg),
        (
            1,
            "Warnings",
            warnings.to_string(),
            "retained buffer".to_string(),
            app.theme.warning,
        ),
        (
            2,
            "Errors",
            errors.to_string(),
            "retained buffer".to_string(),
            app.theme.danger,
        ),
    ] {
        views::metric_tile(
            frame,
            regions[index],
            title,
            value,
            note,
            color,
            &empty,
            app,
        );
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

fn event_detail(entry: Option<&LogEntry>, theme: &Theme) -> Vec<Line<'static>> {
    let Some(entry) = entry else {
        return vec![chrome::placeholder("no event selected", theme)];
    };
    let (_, color) = severity(entry.level, theme, false);
    vec![
        Line::from(chrome::badge(level_name(entry.level), color, theme)),
        chrome::row(
            "App elapsed",
            format!("+{}", crate::util::duration_ms(entry.at.as_millis() as u64)),
            theme.muted,
            12,
            theme,
        ),
        Line::from(""),
        Line::from(Span::styled(
            entry.message.clone(),
            Style::default().fg(theme.fg),
        )),
    ]
}

/// One log row: relative timestamp, severity glyph + name, message.
pub fn line(entry: &LogEntry, theme: &Theme, unicode: bool) -> Line<'static> {
    let (glyph, color) = severity(entry.level, theme, unicode);
    Line::from(vec![
        Span::styled(timestamp(entry), Style::default().fg(theme.border)),
        Span::styled(
            format!(" {glyph} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<6} ", level_name(entry.level)),
            Style::default().fg(color),
        ),
        Span::styled(entry.message.clone(), Style::default().fg(theme.fg)),
    ])
}

/// `MM:SS` since the app started.
pub fn timestamp(entry: &LogEntry) -> String {
    let seconds = entry.at.as_secs();
    format!("{:02}:{:02}", (seconds / 60) % 100, seconds % 60)
}

fn severity(
    level: LogLevel,
    theme: &Theme,
    unicode: bool,
) -> (&'static str, ratatui::style::Color) {
    match (level, unicode) {
        (LogLevel::Info, true) => ("·", theme.info),
        (LogLevel::Warn, true) => ("▲", theme.warning),
        (LogLevel::Error, true) => ("✖", theme.danger),
        (LogLevel::Info, false) => ("-", theme.info),
        (LogLevel::Warn, false) => ("!", theme.warning),
        (LogLevel::Error, false) => ("x", theme.danger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_text(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn event_header_and_inspector_do_not_shift_click_mapping() {
        for width in [80, 120] {
            let mut app = App::new(crate::config::Config::default());
            app.view = View::Log;
            app.snapshot.log = vec![
                entry(LogLevel::Warn, "first event", 10),
                entry(LogLevel::Error, "second event", 20),
            ];
            let text = render_text(&mut app, width, 30);
            let region = app.regions.list.unwrap();
            assert!(
                text.lines()
                    .nth(region.y as usize)
                    .unwrap()
                    .contains("first event")
            );
            assert!(!app.click_row(region.y - 1));
            assert!(app.click_row(region.y + 1));
            assert_eq!(app.selection(View::Log), 1);
            let text = render_text(&mut app, width, 30);
            assert!(text.contains("WARN"));
            assert!(text.contains("ERROR"));
            assert!(text.contains("retained buffer"));
        }
    }

    #[test]
    fn event_inspector_keeps_full_elapsed_time_past_the_row_clock_wrap() {
        let long = entry(LogLevel::Error, "complete detail", 100 * 60 + 5);
        let text = crate::tui::testing::lines_text(&event_detail(Some(&long), theme::FALLBACK));
        assert!(text.contains("+1:40:05"));
        assert!(text.contains("complete detail"));
        assert_eq!(timestamp(&long), "00:05", "row geometry remains stable");
    }

    use crate::tui::testing::line_text;
    use crate::tui::theme;
    use std::time::Duration;

    fn entry(level: LogLevel, message: &str, seconds: u64) -> LogEntry {
        LogEntry {
            level,
            message: message.to_string(),
            at: Duration::from_secs(seconds),
        }
    }

    #[test]
    fn a_row_carries_the_timestamp_and_message() {
        let rendered = line_text(&line(
            &entry(LogLevel::Info, "scene → Live", 65),
            theme::FALLBACK,
            true,
        ));
        assert!(rendered.starts_with("01:05"));
        assert!(rendered.contains("scene → Live"));
    }

    #[test]
    fn severity_is_colour_coded_and_glyphed() {
        let palette = theme::FALLBACK;
        let info = line(&entry(LogLevel::Info, "a", 0), palette, true);
        let warn = line(&entry(LogLevel::Warn, "b", 0), palette, true);
        let error = line(&entry(LogLevel::Error, "c", 0), palette, true);

        assert_eq!(info.spans[1].style.fg, Some(palette.info));
        assert_eq!(warn.spans[1].style.fg, Some(palette.warning));
        assert_eq!(error.spans[1].style.fg, Some(palette.danger));
        assert_ne!(info.spans[1].content, warn.spans[1].content);
    }

    #[test]
    fn ascii_mode_uses_plain_severity_markers() {
        let rendered = line_text(&line(
            &entry(LogLevel::Error, "x", 0),
            theme::FALLBACK,
            false,
        ));
        assert!(rendered.contains(" x "));
        assert!(!rendered.contains('✖'));
    }

    #[test]
    fn timestamps_are_relative_and_zero_padded() {
        assert_eq!(timestamp(&entry(LogLevel::Info, "a", 0)), "00:00");
        assert_eq!(timestamp(&entry(LogLevel::Info, "a", 9)), "00:09");
        assert_eq!(timestamp(&entry(LogLevel::Info, "a", 61)), "01:01");
    }

    #[test]
    fn timestamps_wrap_rather_than_widening_the_column() {
        // A session running past 100 minutes must not push the column out and
        // reflow every row.
        let long = timestamp(&entry(LogLevel::Info, "a", 100 * 60 + 5));
        assert_eq!(long.len(), 5);
        assert_eq!(long, "00:05");
    }
}

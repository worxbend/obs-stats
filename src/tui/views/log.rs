//! The Log view — OBS events since this session started.
//!
//! Timestamps are relative to app start rather than wall-clock: the question
//! this view answers is "how long before the drop did the scene change?", and
//! a relative clock answers it without arithmetic.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::LogEntry;
use crate::obs::client::LogLevel;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
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

    let [context_area, list_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).areas(area);

    let context_block = chrome::panel("Now", false, theme, app.unicode);
    let context_inner = chrome::frame_panel(frame, context_area, context_block);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "scene · profile · collection   ",
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
        format!("{}", entries.len()),
        true,
        theme,
        app.unicode,
    );
    let inner = chrome::frame_panel(frame, list_area, block);

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
    );
    app.put_list_state(View::Log, inner, state);
}

/// One log row: relative timestamp, severity glyph, message.
pub fn line(entry: &LogEntry, theme: &Theme, unicode: bool) -> Line<'static> {
    let (glyph, color) = severity(entry.level, theme, unicode);
    Line::from(vec![
        Span::styled(timestamp(entry), Style::default().fg(theme.border)),
        Span::styled(
            format!(" {glyph} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
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

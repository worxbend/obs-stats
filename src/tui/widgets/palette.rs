//! The fuzzy command palette, in the shape Telescope users expect: a query
//! line at the top, ranked matches below, the best one preselected.
//!
//! Rows come from [`crate::tui::command::palette_items`], the same registry the
//! `:` line parses against, so anything runnable by name is findable here.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};

use crate::tui::app::App;
use crate::tui::command::PaletteItem;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Rows shown at once before the list scrolls.
const VISIBLE_ROWS: u16 = 12;

/// Draw the palette, returning the panel and the row list so a click can be
/// resolved against them.
pub fn render(frame: &mut Frame, area: Rect, app: &App, state: &mut ListState) -> (Rect, Rect) {
    // +2 border, +2 for the query row and its rule.
    let height = (VISIBLE_ROWS + 4).min(area.height);
    let region = chrome::centered(area, 64.min(area.width), height);
    frame.render_widget(Clear, region);

    let block = chrome::panel_with_note(
        app.palette.title(),
        format!("{}", app.palette.matches.len()),
        true,
        app.theme,
        app.unicode,
    )
    .style(Style::default().bg(app.theme.bg));

    let inner = block.inner(region);
    frame.render_widget(block, region);

    if inner.height == 0 {
        return (region, inner);
    }

    let [query_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    frame.render_widget(Paragraph::new(query_line(app)), query_area);

    if app.palette.matches.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder("no matches", app.theme)),
            list_area,
        );
        return (region, list_area);
    }

    let width = list_area.width as usize;
    let items: Vec<ListItem> = app
        .palette
        .matches
        .iter()
        .map(|item| ListItem::new(row(item, width, app.theme)))
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(app.theme.highlight_bg)
            .fg(app.theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    );

    state.select(Some(app.palette.cursor));
    frame.render_stateful_widget(list, list_area, state);

    (region, list_area)
}

/// `> query` with a real cursor at the end.
fn query_line(app: &App) -> Line<'static> {
    let theme = app.theme;
    let prompt = if app.unicode { "❯ " } else { "> " };
    Line::from(vec![
        Span::styled(
            prompt,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.palette.query.clone(), Style::default().fg(theme.fg)),
    ])
}

/// One match: its label, then its description dimmed on the right.
pub fn row(item: &PaletteItem, width: usize, theme: &Theme) -> Line<'static> {
    // Reserve a third of the row for the detail, so a long label truncates
    // rather than pushing the description off the panel.
    let detail_width = (width / 3).max(8);
    let label_width = width.saturating_sub(detail_width + 2).max(6);

    Line::from(vec![
        Span::styled(
            format!(
                " {:<width$}",
                crate::util::truncate(&item.label, label_width),
                width = label_width
            ),
            Style::default().fg(theme.fg),
        ),
        Span::styled(
            crate::util::truncate(&item.detail, detail_width),
            Style::default().fg(theme.muted),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::keys::Action;
    use crate::tui::theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn app_with(filter: &'static str, query: &str) -> App {
        let mut app = App::new(Config::default());
        app.handle(Action::OpenPaletteFiltered(filter), Instant::now());
        for c in query.chars() {
            app.handle(Action::Input(c), Instant::now());
        }
        app
    }

    fn render_at(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), app, &mut ListState::default());
            })
            .unwrap();
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

    #[test]
    fn the_palette_lists_matches_under_the_query() {
        let app = app_with("", "audio");
        let rendered = render_at(&app, 100, 30);
        assert!(rendered.contains("audio"), "{rendered}");
        assert!(rendered.contains("Commands"), "the panel title");
    }

    #[test]
    fn a_filtered_palette_says_what_it_is_filtered_to() {
        let app = app_with("theme", "");
        assert!(render_at(&app, 100, 30).contains("Themes"));

        let views = app_with("view", "");
        assert!(render_at(&views, 100, 30).contains("Views"));
    }

    #[test]
    fn an_empty_result_says_so_rather_than_drawing_a_blank_panel() {
        let app = app_with("", "zzzzzz");
        let rendered = render_at(&app, 100, 30);
        assert!(rendered.contains("no matches"), "{rendered}");
    }

    #[test]
    fn the_row_under_the_cursor_is_highlighted() {
        let app = app_with("view", "");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &app, &mut ListState::default());
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        let highlighted = (0..buffer.area.height)
            .any(|y| (0..buffer.area.width).any(|x| buffer[(x, y)].bg == app.theme.highlight_bg));
        assert!(highlighted, "the preselected match must be visible as such");
    }

    #[test]
    fn rows_pad_the_label_so_the_details_form_a_column() {
        let item = PaletteItem {
            label: "view log".into(),
            detail: "Events reported by OBS".into(),
            kind: "view",
            action: Action::Quit,
        };
        let short = row(&item, 60, theme::FALLBACK);
        let width: usize = short
            .spans
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        assert!(width <= 60, "a row must not overflow its panel");
        assert!(short.spans[0].content.starts_with(" view log"));
    }

    #[test]
    fn a_long_label_and_detail_are_both_truncated() {
        let item = PaletteItem {
            label: "a".repeat(200),
            detail: "b".repeat(200),
            kind: "view",
            action: Action::Quit,
        };
        let line = row(&item, 40, theme::FALLBACK);
        let width: usize = line
            .spans
            .iter()
            .map(|span| span.content.chars().count())
            .sum();
        assert!(width <= 40, "{width} columns in a 40-column panel");
    }

    #[test]
    fn the_reported_regions_are_the_panel_and_the_rows() {
        let app = app_with("view", "");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut regions = (Rect::default(), Rect::default());
        terminal
            .draw(|frame| {
                regions = render(frame, frame.area(), &app, &mut ListState::default());
            })
            .unwrap();
        let (panel, list) = regions;
        assert!(panel.width > 0 && panel.height > 0);
        // The list sits inside the panel, below its border and query row.
        assert!(list.y > panel.y);
        assert!(list.bottom() <= panel.bottom());
    }

    #[test]
    fn a_cramped_terminal_does_not_panic() {
        for (width, height) in [(20, 6), (30, 4), (10, 20)] {
            let app = app_with("", "e");
            let _ = render_at(&app, width, height);
        }
    }

    #[test]
    fn ascii_mode_uses_no_drawing_glyphs() {
        let mut config = Config::default();
        config.ui.unicode = false;
        let mut app = App::new(config);
        app.handle(Action::OpenPalette, Instant::now());

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &app, &mut ListState::default());
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                for c in buffer[(x, y)].symbol().chars() {
                    assert!(
                        !is_drawing_glyph(c),
                        "{c:?} (U+{:04X}) at ({x},{y}) is a glyph --ascii avoids",
                        c as u32
                    );
                }
            }
        }
    }

    /// The ranges `--ascii` exists to avoid, matching the sweep in
    /// [`crate::tui::ui`]. General Punctuation — the `…` a truncated label
    /// ends in — is deliberately not among them.
    fn is_drawing_glyph(c: char) -> bool {
        matches!(c as u32,
            0x2190..=0x21FF
            | 0x2500..=0x257F
            | 0x2580..=0x259F
            | 0x25A0..=0x25FF
            | 0x1F000..=0x1FAFF
        )
    }
}

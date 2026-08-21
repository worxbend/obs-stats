//! The theme picker overlay, in the style of btop's.
//!
//! The list is only half the widget: because [`App`] restyles itself as the
//! cursor moves, the *entire* dashboard behind the overlay is the preview.
//! Enter keeps the previewed theme, Esc puts the old one back.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};

use crate::tui::app::App;
use crate::tui::theme::{self, Theme};
use crate::tui::widgets::chrome;

/// Rows of themes shown at once, before the list scrolls.
const VISIBLE_ROWS: u16 = 12;

/// Draw the picker, returning the panel and the row list so a click can be
/// resolved against them.
pub fn render(frame: &mut Frame, area: Rect, app: &App, state: &mut ListState) -> (Rect, Rect) {
    // +2 border, +2 for the footer hint and its spacer.
    let height = (VISIBLE_ROWS + 4).min(area.height);
    let region = chrome::centered(area, 52.min(area.width), height);
    frame.render_widget(Clear, region);

    let block = chrome::panel_with_note(
        "Appearance",
        format!("{}/{}", app.theme_cursor + 1, theme::THEMES.len()),
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

    // Reserve the last row for the footer, but only if there is room for a
    // list as well — in a tiny terminal the list matters more.
    let footer_height = u16::from(inner.height > 2);
    let list_area = Rect {
        height: inner.height - footer_height,
        ..inner
    };

    let items: Vec<ListItem> = theme::THEMES
        .iter()
        .map(|entry| ListItem::new(row(entry, app.theme, app.unicode)))
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(app.theme.highlight_bg)
            .fg(app.theme.highlight_fg)
            .add_modifier(Modifier::BOLD),
    );

    state.select(Some(app.theme_cursor));
    frame.render_stateful_widget(list, list_area, state);

    if footer_height == 1 {
        let footer = Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter applies · Esc reverts",
                Style::default().fg(app.theme.muted),
            ))),
            footer,
        );
    }

    (region, list_area)
}

/// One picker row: swatches, then the label.
///
/// The swatches are drawn in the *candidate* theme's colours, not the active
/// one, so the list doubles as a palette comparison.
pub fn row(entry: &Theme, active: &Theme, unicode: bool) -> Line<'static> {
    let glyphs = chrome::glyphs(unicode);
    let mut spans = vec![Span::raw(" ")];

    for color in [
        entry.accent,
        entry.success,
        entry.warning,
        entry.danger,
        entry.info,
    ] {
        spans.push(Span::styled(
            glyphs.bar_full.to_string(),
            Style::default().fg(color),
        ));
    }

    spans.push(Span::raw("  "));
    let is_active = entry.id == active.id;
    spans.push(Span::styled(
        entry.label.to_string(),
        if is_active {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    ));

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::testing::line_text;

    #[test]
    fn a_row_shows_swatches_then_the_label() {
        let entry = theme::find("nord").unwrap();
        let line = row(entry, theme::FALLBACK, true);
        let rendered = line_text(&line);
        assert!(rendered.contains("Nord"));
        assert_eq!(rendered.matches('█').count(), 5, "five swatches");
    }

    #[test]
    fn swatches_use_the_candidate_theme_not_the_active_one() {
        let nord = theme::find("nord").unwrap();
        let line = row(nord, theme::FALLBACK, true);
        assert_eq!(line.spans[1].style.fg, Some(nord.accent));
        assert_ne!(line.spans[1].style.fg, Some(theme::FALLBACK.accent));
    }

    #[test]
    fn the_active_theme_is_emphasised_in_the_list() {
        let active = theme::FALLBACK;
        let own_row = row(active, active, true);
        let other_row = row(theme::find("nord").unwrap(), active, true);
        let label_of = |line: &Line<'_>| line.spans.last().unwrap().style;
        assert!(label_of(&own_row).add_modifier.contains(Modifier::BOLD));
        assert!(!label_of(&other_row).add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn ascii_mode_swaps_the_swatch_glyph() {
        let line = row(theme::FALLBACK, theme::FALLBACK, false);
        let rendered = line_text(&line);
        assert!(rendered.contains('#'));
        assert!(!rendered.contains('█'));
    }

    #[test]
    fn every_theme_produces_a_labelled_row() {
        for entry in theme::THEMES {
            assert!(
                line_text(&row(entry, theme::FALLBACK, true)).contains(entry.label),
                "{} rendered without its label",
                entry.id
            );
        }
    }
}

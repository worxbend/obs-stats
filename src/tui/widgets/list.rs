//! One rendering path for every selectable list.
//!
//! Five lists in this UI are navigable with a cursor: scenes, audio inputs,
//! the log, the theme picker and the command palette. They differ only in
//! what a row looks like. Everything after that — the highlight style, telling
//! the widget where the cursor is, and rendering statefully so the scroll
//! offset survives the frame — is the same in all five, and used to be
//! written out five times.
//!
//! Keeping it in one place is what makes "selected rows look the same
//! everywhere" a property of the code rather than of five copies staying in
//! agreement.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState};

use crate::tui::theme::Theme;

/// How the row under the cursor is drawn.
pub fn highlight_style(theme: &Theme) -> Style {
    Style::default()
        .bg(theme.highlight_bg)
        .fg(theme.highlight_fg)
        .add_modifier(Modifier::BOLD)
}

/// Draw `rows` into `area` with `cursor` highlighted.
///
/// Rendered statefully so `state` keeps the scroll offset: it is what the
/// next frame scrolls from, and what a click is resolved against.
pub fn render_rows(
    frame: &mut Frame,
    area: Rect,
    rows: Vec<Line<'static>>,
    cursor: usize,
    state: &mut ListState,
    theme: &Theme,
) {
    let items: Vec<ListItem> = rows.into_iter().map(ListItem::new).collect();
    let list = List::new(items).highlight_style(highlight_style(theme));
    state.select(Some(cursor));
    frame.render_stateful_widget(list, area, state);
}

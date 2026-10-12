//! The which-key popup, in the style AstroNvim shows after the leader.
//!
//! It docks at the bottom of the screen and lists everything reachable from
//! the prefix currently being typed. The rows come straight from
//! [`crate::tui::keys::menu`], so a binding that exists is a binding that is
//! advertised — the popup cannot fall out of step with the keymap.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::keys::{self, Entry};
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;

/// Columns one entry occupies: `key  label`, padded.
const COLUMN_WIDTH: u16 = 24;

/// Most rows the popup will grow to before it starts using more columns.
const MAX_ROWS: u16 = 8;

/// Draw the popup for whatever prefix is pending, if any.
pub fn render(frame: &mut Frame, area: Rect, app: &App) -> Option<Rect> {
    let (title, entries) = keys::menu(app.pending)?;
    if entries.is_empty() {
        return None;
    }

    let columns = columns_for(area.width, entries.len());
    let rows = entries.len().div_ceil(columns) as u16;

    // +2 for the border. Anchored to the bottom, above the status bar, which
    // is where the eye already is after pressing the leader.
    let height = (rows + 2).min(area.height);
    let width = ((columns as u16 * COLUMN_WIDTH) + 2).min(area.width);
    let region = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };

    let block = chrome::panel_with_note(
        title,
        format!("{} keys", entries.len()),
        true,
        app.theme,
        app.unicode,
    );
    let inner = chrome::popup(frame, region, block, app.theme);

    if inner.height > 0 {
        frame.render_widget(
            Paragraph::new(grid(entries, columns, app.theme, app.unicode)),
            inner,
        );
    }
    Some(region)
}

/// How many columns to spread `count` entries over inside `width`.
///
/// Grows the column count rather than the row count, so a long menu stays a
/// squat block at the bottom instead of a tower up the screen.
fn columns_for(width: u16, count: usize) -> usize {
    let fits = ((width.saturating_sub(2)) / COLUMN_WIDTH).max(1) as usize;
    let needed = (count as u16).div_ceil(MAX_ROWS) as usize;
    needed.clamp(1, fits).min(count)
}

/// Lay the entries out in column-major order, so reading down a column is
/// reading consecutive keys.
fn grid(
    entries: &'static [Entry],
    columns: usize,
    theme: &Theme,
    unicode: bool,
) -> Vec<Line<'static>> {
    let rows = entries.len().div_ceil(columns);
    let glyphs = chrome::glyphs(unicode);

    (0..rows)
        .map(|row| {
            let mut spans = Vec::new();
            for column in 0..columns {
                let Some(entry) = entries.get(column * rows + row) else {
                    continue;
                };
                spans.extend(cell(entry, glyphs.arrow, theme));
            }
            Line::from(spans)
        })
        .collect()
}

/// One `key → label` cell, padded to [`COLUMN_WIDTH`].
///
/// Groups are marked with a trailing separator, as which-key does, so a key
/// that opens another menu is distinguishable from one that acts.
fn cell(entry: &'static Entry, arrow: &str, theme: &Theme) -> Vec<Span<'static>> {
    let label = if entry.is_group() {
        format!("+{}", entry.label())
    } else {
        entry.label().to_string()
    };

    // 3 for `" k "`, then the arrow and the space after it — the rest is the
    // label's, truncated so the columns stay aligned.
    let label_width = (COLUMN_WIDTH as usize).saturating_sub(3 + arrow.chars().count() + 1);
    let color = if entry.is_group() {
        theme.accent_alt
    } else {
        theme.fg
    };

    let mut spans = chrome::kbd(&entry.key().to_string(), theme);
    spans.push(Span::styled(
        format!("{arrow} "),
        Style::default().fg(theme.border),
    ));
    spans.push(Span::styled(
        format!(
            "{:<width$}",
            crate::util::truncate(&label, label_width),
            width = label_width
        ),
        Style::default().fg(color),
    ));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::keys::Pending;
    use crate::tui::theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_at(pending: Pending, width: u16, height: u16) -> String {
        let mut app = App::new(Config::default());
        app.pending = pending;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &app);
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
    fn nothing_is_drawn_when_no_sequence_is_pending() {
        let mut app = App::new(Config::default());
        app.pending = Pending::None;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut region = Some(Rect::default());
        terminal
            .draw(|frame| {
                region = render(frame, frame.area(), &app);
            })
            .unwrap();
        assert!(region.is_none());
    }

    #[test]
    fn the_leader_menu_lists_its_groups_and_bindings() {
        let rendered = render_at(Pending::Leader, 100, 24);
        assert!(rendered.contains("Quit"), "{rendered}");
        assert!(rendered.contains("+Find"), "groups are marked with a +");
        assert!(rendered.contains("+UI toggles"));
    }

    #[test]
    fn a_group_menu_lists_only_that_group() {
        let rendered = render_at(Pending::LeaderGroup('u'), 100, 24);
        assert!(rendered.contains("Unicode drawing"));
        assert!(!rendered.contains("+Find"), "{rendered}");
    }

    #[test]
    fn the_g_prefix_has_its_own_menu() {
        let rendered = render_at(Pending::Prefix('g'), 100, 24);
        assert!(rendered.contains("Top of the list"), "{rendered}");
    }

    #[test]
    fn every_entry_reaches_the_screen() {
        // A popup that silently drops bindings would advertise a keymap the
        // app does not have.
        let (_, entries) = keys::menu(Pending::Leader).unwrap();
        let rendered = render_at(Pending::Leader, 120, 30);
        for entry in entries {
            assert!(
                rendered.contains(entry.label())
                    || rendered.contains(&entry.label()[..6.min(entry.label().len())]),
                "{} is missing from the popup:\n{rendered}",
                entry.label()
            );
        }
    }

    #[test]
    fn the_layout_grows_columns_rather_than_rows() {
        // Nine entries at eight rows maximum needs two columns.
        assert_eq!(columns_for(200, 9), 2);
        assert_eq!(columns_for(200, 8), 1);
        // ...but never more columns than the terminal has room for.
        assert_eq!(columns_for(30, 40), 1);
        assert_eq!(columns_for(0, 40), 1);
        assert_eq!(columns_for(200, 1), 1, "never more columns than entries");
    }

    #[test]
    fn cells_are_padded_to_a_fixed_width_so_columns_line_up() {
        let (_, entries) = keys::menu(Pending::Leader).unwrap();
        for line in grid(entries, 2, theme::FALLBACK, true) {
            let width: usize = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum();
            assert_eq!(width % COLUMN_WIDTH as usize, 0, "ragged row: {line:?}");
        }
    }

    #[test]
    fn ascii_mode_uses_no_drawing_glyphs() {
        let rendered = render_at(Pending::Leader, 100, 24);
        let _ = rendered;

        let mut config = Config::default();
        config.ui.unicode = false;
        let mut app = App::new(config);
        app.pending = Pending::Leader;

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &app);
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
    /// [`crate::tui::ui`].
    fn is_drawing_glyph(c: char) -> bool {
        matches!(c as u32,
            0x2190..=0x21FF
            | 0x2500..=0x257F
            | 0x2580..=0x259F
            | 0x25A0..=0x25FF
            | 0x1F000..=0x1FAFF
        )
    }

    #[test]
    fn the_popup_survives_a_terminal_too_small_to_hold_it() {
        for (width, height) in [(20, 6), (10, 4), (40, 3)] {
            let _ = render_at(Pending::Leader, width, height);
        }
    }

    #[test]
    fn a_long_label_is_truncated_rather_than_breaking_the_column() {
        let spans = cell(
            &Entry::Leaf {
                key: 'x',
                label: "An extremely long label that will not fit in a cell",
                action: crate::tui::keys::Action::Quit,
            },
            "→",
            theme::FALLBACK,
        );
        let width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        assert_eq!(width, COLUMN_WIDTH as usize);
    }
}

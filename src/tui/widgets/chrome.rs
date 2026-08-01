//! Shared panel furniture: borders, glyph sets, bars and label/value rows.
//!
//! Everything here takes an explicit `unicode` flag rather than reading it
//! from app state, so the fallback path (`u` at runtime, or `ui.unicode:
//! false` for a Linux VT) is exercised by the same code as the pretty path.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};

use crate::tui::theme::{self, Theme};

/// The glyphs a panel draws with, chosen by the Unicode setting.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    /// Filled and empty cells of a horizontal bar.
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
    /// Filled and empty cells of a segmented meter.
    pub meter_full: &'static str,
    pub meter_empty: &'static str,
    /// Bullet before a list row.
    pub bullet: &'static str,
    /// Marker for the live/recording indicator.
    pub dot: &'static str,
    /// Arrow used in "a → b" summaries.
    pub arrow: &'static str,
    /// Separator between status-bar fields.
    pub separator: &'static str,
}

pub const UNICODE_GLYPHS: Glyphs = Glyphs {
    bar_full: "█",
    bar_empty: "░",
    meter_full: "▰",
    meter_empty: "▱",
    bullet: "•",
    dot: "●",
    arrow: "→",
    separator: "│",
};

pub const ASCII_GLYPHS: Glyphs = Glyphs {
    bar_full: "#",
    bar_empty: ".",
    meter_full: "#",
    meter_empty: "-",
    bullet: "*",
    dot: "*",
    arrow: "->",
    separator: "|",
};

pub fn glyphs(unicode: bool) -> Glyphs {
    if unicode {
        UNICODE_GLYPHS
    } else {
        ASCII_GLYPHS
    }
}

/// Border drawn in ASCII mode.
///
/// `BorderType::Plain` is *not* plain ASCII — it is U+2500 box drawing, the
/// exact range `--ascii` exists to avoid. Every other border style ratatui
/// ships is Unicode too, so the fallback needs its own set.
const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// A bordered panel.
///
/// The focused panel gets a thick border in the accent colour; everything
/// else gets a plain one, so focus is legible without colour alone.
pub fn panel<'a>(
    title: impl Into<String>,
    focused: bool,
    theme: &Theme,
    unicode: bool,
) -> Block<'a> {
    let border_color = if focused {
        theme.border_focus
    } else {
        theme.border
    };
    let title_style = Style::default()
        .fg(if focused { theme.accent } else { theme.muted })
        .add_modifier(Modifier::BOLD);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(format!(" {} ", title.into()), title_style))
        .padding(Padding::horizontal(1));

    if unicode {
        // Focus is signalled by weight as well as colour, so it survives a
        // monochrome terminal.
        block.border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
    } else {
        block.border_set(ASCII_BORDER)
    }
}

/// A panel with a right-aligned subtitle in the top border.
pub fn panel_with_note<'a>(
    title: impl Into<String>,
    note: impl Into<String>,
    focused: bool,
    theme: &Theme,
    unicode: bool,
) -> Block<'a> {
    panel(title, focused, theme, unicode).title_top(
        Line::from(Span::styled(
            format!(" {} ", note.into()),
            Style::default().fg(theme.muted),
        ))
        .alignment(Alignment::Right),
    )
}

/// A horizontal bar of `width` cells filled to `ratio`, coloured by `color`.
///
/// The fill rounds down, so a non-zero ratio always shows at least one cell
/// once it exceeds one cell's worth — a bar that reads empty while the number
/// beside it reads 4% is worse than a slightly generous bar.
pub fn bar(
    ratio: f64,
    width: usize,
    color: Color,
    theme: &Theme,
    unicode: bool,
) -> Vec<Span<'static>> {
    let glyphs = glyphs(unicode);
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((ratio * width as f64).round() as usize).min(width);

    let mut spans = Vec::with_capacity(2);
    if filled > 0 {
        spans.push(Span::styled(
            glyphs.bar_full.repeat(filled),
            Style::default().fg(color),
        ));
    }
    if filled < width {
        spans.push(Span::styled(
            glyphs.bar_empty.repeat(width - filled),
            Style::default().fg(theme.border),
        ));
    }
    spans
}

/// A bar whose colour runs along a gradient as it fills — the btop look.
///
/// The gradient runs success → warning → danger, so a long bar tells you how
/// bad things are from its colour alone, without reading the number.
pub fn gradient_bar(ratio: f64, width: usize, theme: &Theme, unicode: bool) -> Vec<Span<'static>> {
    let glyphs = glyphs(unicode);
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((ratio * width as f64).round() as usize).min(width);

    let mut spans = Vec::with_capacity(width);
    for cell in 0..filled {
        // Colour by the cell's own position, not the overall ratio, so the
        // gradient stays anchored to the scale rather than sliding about.
        let position = if width <= 1 {
            0.0
        } else {
            cell as f32 / (width - 1) as f32
        };
        let color = if position <= 0.5 {
            theme::blend(theme.success, theme.warning, position * 2.0)
        } else {
            theme::blend(theme.warning, theme.danger, (position - 0.5) * 2.0)
        };
        spans.push(Span::styled(glyphs.bar_full, Style::default().fg(color)));
    }
    if filled < width {
        spans.push(Span::styled(
            glyphs.bar_empty.repeat(width - filled),
            Style::default().fg(theme.border),
        ));
    }
    spans
}

/// A `label   value` row, with the label muted and the value coloured.
pub fn row(
    label: &str,
    value: impl Into<String>,
    color: Color,
    label_width: usize,
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<label_width$}"),
            Style::default().fg(theme.muted),
        ),
        Span::styled(value.into(), Style::default().fg(color)),
    ])
}

/// A `label  [bar]  value` row — the workhorse of the overview deck.
///
/// Every parameter is a distinct visual decision the caller has already made;
/// bundling them into a struct would only move the same list one line up.
#[allow(clippy::too_many_arguments)]
pub fn gauge_row(
    label: &str,
    ratio: f64,
    value: impl Into<String>,
    color: Color,
    label_width: usize,
    bar_width: usize,
    theme: &Theme,
    unicode: bool,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{label:<label_width$}"),
        Style::default().fg(theme.muted),
    )];
    spans.extend(bar(ratio, bar_width, color, theme, unicode));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        value.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

/// A pill-shaped status chip, e.g. `● LIVE`.
pub fn chip(text: &str, color: Color, active: bool, unicode: bool) -> Vec<Span<'static>> {
    let glyphs = glyphs(unicode);
    let style = if active {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    vec![
        Span::styled(format!("{} ", glyphs.dot), style),
        Span::styled(text.to_string(), style),
    ]
}

/// Centre `area`'s inner region to `width` × `height`, clamped to fit.
///
/// Used to place the modal overlays. Returning a clamped rect rather than
/// panicking keeps the app usable in a very small terminal.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Text shown in place of a panel's contents when there is nothing to show.
pub fn placeholder(message: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        message.to_string(),
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::ITALIC),
    ))
    .alignment(Alignment::Center)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> &'static Theme {
        theme::FALLBACK
    }

    /// Total display width of a span list.
    fn width(spans: &[Span<'_>]) -> usize {
        spans.iter().map(|span| span.content.chars().count()).sum()
    }

    #[test]
    fn glyph_sets_switch_on_the_unicode_flag() {
        assert_eq!(glyphs(true).bar_full, "█");
        assert_eq!(glyphs(false).bar_full, "#");
        assert_eq!(glyphs(false).arrow, "->");
    }

    #[test]
    fn ascii_glyphs_are_all_single_byte() {
        // The whole point of the fallback set is that it renders on a VT.
        for glyph in [
            ASCII_GLYPHS.bar_full,
            ASCII_GLYPHS.bar_empty,
            ASCII_GLYPHS.meter_full,
            ASCII_GLYPHS.meter_empty,
            ASCII_GLYPHS.bullet,
            ASCII_GLYPHS.dot,
            ASCII_GLYPHS.separator,
        ] {
            assert!(glyph.is_ascii(), "{glyph} is not ASCII");
        }
    }

    #[test]
    fn a_bar_always_occupies_its_full_width() {
        for ratio in [0.0, 0.01, 0.5, 0.99, 1.0] {
            let spans = bar(ratio, 20, Color::Red, theme(), true);
            assert_eq!(width(&spans), 20, "ratio {ratio} produced the wrong width");
        }
    }

    #[test]
    fn a_full_bar_has_no_empty_cells_and_an_empty_bar_has_no_filled_ones() {
        let full = bar(1.0, 10, Color::Red, theme(), true);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].content.chars().count(), 10);

        let empty = bar(0.0, 10, Color::Red, theme(), true);
        assert_eq!(empty.len(), 1);
        assert!(empty[0].content.starts_with('░'));
    }

    #[test]
    fn bars_clamp_out_of_range_and_non_finite_ratios() {
        assert_eq!(width(&bar(5.0, 10, Color::Red, theme(), true)), 10);
        assert_eq!(width(&bar(-1.0, 10, Color::Red, theme(), true)), 10);
        let nan = bar(f64::NAN, 10, Color::Red, theme(), true);
        assert_eq!(width(&nan), 10);
        assert!(nan[0].content.starts_with('░'), "NaN must read as empty");
    }

    #[test]
    fn a_zero_width_bar_produces_nothing() {
        assert!(bar(0.5, 0, Color::Red, theme(), true).is_empty());
        assert!(gradient_bar(0.5, 0, theme(), true).is_empty());
    }

    #[test]
    fn a_gradient_bar_also_occupies_its_full_width() {
        for ratio in [0.0, 0.25, 0.5, 1.0] {
            let spans = gradient_bar(ratio, 16, theme(), true);
            assert_eq!(width(&spans), 16, "ratio {ratio} produced the wrong width");
        }
    }

    #[test]
    fn a_gradient_bar_runs_from_success_to_danger() {
        let spans = gradient_bar(1.0, 16, theme(), true);
        let first = spans.first().unwrap().style.fg.unwrap();
        let last = spans[15].style.fg.unwrap();
        assert_eq!(first, theme().success);
        assert_eq!(last, theme().danger);
    }

    #[test]
    fn a_gradient_bar_of_one_cell_does_not_divide_by_zero() {
        let spans = gradient_bar(1.0, 1, theme(), true);
        assert_eq!(width(&spans), 1);
        assert_eq!(spans[0].style.fg.unwrap(), theme().success);
    }

    #[test]
    fn label_columns_are_padded_to_a_fixed_width() {
        let line = row("CPU", "12%", Color::Green, 10, theme());
        assert_eq!(line.spans[0].content, "CPU       ");
        assert_eq!(line.spans[1].content, "12%");
    }

    #[test]
    fn a_label_longer_than_its_column_is_not_truncated() {
        // Better to push the value right than to render a mystery label.
        let line = row("A VERY LONG LABEL", "1", Color::Green, 4, theme());
        assert_eq!(line.spans[0].content, "A VERY LONG LABEL");
    }

    #[test]
    fn gauge_rows_carry_label_bar_and_value() {
        let line = gauge_row("CPU", 0.5, "50%", Color::Green, 8, 10, theme(), true);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("CPU     "));
        assert!(text.ends_with("50%"));
        assert!(text.contains('█'));
    }

    #[test]
    fn chips_are_bolder_when_active() {
        let active = chip("LIVE", Color::Red, true, true);
        let idle = chip("LIVE", Color::Red, false, true);
        assert!(active[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(!idle[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(active[0].content, "● ");
        assert_eq!(chip("LIVE", Color::Red, true, false)[0].content, "* ");
    }

    #[test]
    fn centering_places_a_box_in_the_middle() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        let centered = centered(area, 40, 20);
        assert_eq!(centered.width, 40);
        assert_eq!(centered.height, 20);
        assert_eq!(centered.x, 30);
        assert_eq!(centered.y, 15);
    }

    #[test]
    fn centering_clamps_to_a_terminal_too_small_for_the_box() {
        let area = Rect {
            x: 2,
            y: 3,
            width: 20,
            height: 10,
        };
        let centered = centered(area, 80, 40);
        assert_eq!(centered.width, 20);
        assert_eq!(centered.height, 10);
        assert_eq!(centered.x, 2);
        assert_eq!(centered.y, 3);
        // And it must stay inside the parent.
        assert!(centered.right() <= area.right());
        assert!(centered.bottom() <= area.bottom());
    }

    #[test]
    fn the_ascii_border_set_is_actually_ascii() {
        // ratatui's "Plain" border is U+2500 box drawing, so the fallback has
        // to bring its own set — this is the guard against regressing to it.
        for glyph in [
            ASCII_BORDER.top_left,
            ASCII_BORDER.top_right,
            ASCII_BORDER.bottom_left,
            ASCII_BORDER.bottom_right,
            ASCII_BORDER.vertical_left,
            ASCII_BORDER.vertical_right,
            ASCII_BORDER.horizontal_top,
            ASCII_BORDER.horizontal_bottom,
        ] {
            assert!(glyph.is_ascii(), "{glyph} is not ASCII");
        }
    }

    #[test]
    fn a_focused_panel_uses_the_accent_border() {
        // Exercised via the public builder to keep the styling rule pinned.
        let focused = panel("Test", true, theme(), true);
        let unfocused = panel("Test", false, theme(), true);
        assert_ne!(
            format!("{focused:?}"),
            format!("{unfocused:?}"),
            "focus must change the panel's appearance"
        );
    }
}

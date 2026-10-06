//! Shared panel furniture: borders, glyph sets, bars, badges and label/value
//! rows — the design system's component layer.
//!
//! Everything here takes an explicit `unicode` flag rather than reading it
//! from app state, so the fallback path (`u` at runtime, or `ui.unicode:
//! false` for a Linux VT) is exercised by the same code as the pretty path.
//!
//! Colour decisions go through the theme's derived tokens (`surface`,
//! `track`, `fade`, `gradient`, `on_color`), never a literal `Color`, so a
//! new palette restyles every component at once.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding};

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

/// Sub-cell fill steps, eighths of a cell in ascending order.
///
/// These are Block Elements, so they belong to the Unicode path only; the
/// ASCII fallback fills whole cells, as it always has.
const PARTIALS: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// The colour of a bar's empty cells: the raised `track` token on a painted
/// theme, the plain border colour on a flat one (whose `track` is
/// `Color::Reset` and would render as the terminal's default foreground).
pub fn track_color(theme: &Theme) -> Color {
    if theme.flat() {
        theme.border
    } else {
        theme.track()
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

/// A bordered panel, filled with the theme's `surface` elevation.
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
        // The surface tint is what makes a panel read as a card sitting on
        // the page rather than as a rectangle drawn around text. On a flat
        // theme this is `Color::Reset` and costs nothing.
        .style(Style::default().bg(theme.surface()))
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

/// A panel whose top-right note is a coloured badge — for states like
/// `LIVE`, `idle` or `paused` that deserve more than dimmed text.
pub fn panel_with_badge<'a>(
    title: impl Into<String>,
    note: &str,
    note_color: Color,
    focused: bool,
    theme: &Theme,
    unicode: bool,
) -> Block<'a> {
    let mut spans = badge(note, note_color, theme);
    spans.push(Span::raw(" "));
    panel(title, focused, theme, unicode).title_top(Line::from(spans).alignment(Alignment::Right))
}

/// Draw a panel's frame and return the area left inside it.
///
/// Every panel does the same two steps in the same order: work out the inner
/// rect *before* rendering, because `render_widget` consumes the block. Doing
/// it the other way round does not compile, but writing both lines out at 24
/// call sites is 24 chances to reverse them.
///
/// Callers still check `inner.height == 0` themselves — a panel squeezed to
/// its borders has nowhere to draw, and what each one does about that
/// differs.
pub fn frame_panel(frame: &mut Frame, area: Rect, block: Block<'_>) -> Rect {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Draw an overlay's frame over whatever was beneath it, returning the area
/// left inside.
///
/// The `.style(bg(theme.surface_hi()))` here is load-bearing and easy to
/// drop. `Clear` does not paint the theme's background — it resets the cells
/// it covers, which on a terminal with a transparent or image background
/// means the user's own wallpaper shows through the middle of the overlay.
/// The restyle is what puts a background back, one elevation step above the
/// panels so the modal reads as floating.
///
/// `no_widget_punches_a_hole_in_the_themed_background` is the test that
/// catches forgetting it, and it sweeps every overlay for exactly this
/// reason.
pub fn popup(frame: &mut Frame, region: Rect, block: Block<'_>, theme: &Theme) -> Rect {
    frame.render_widget(Clear, region);
    frame_panel(
        frame,
        region,
        block.style(Style::default().bg(theme.surface_hi())),
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
            Style::default().fg(track_color(theme)),
        ));
    }
    spans
}

/// The success → warning → danger colour at `position` along a scale.
///
/// Shared by the gradient bar and the sub-cell meter so both tell the same
/// colour story: how bad things are, readable from the bar alone.
fn gradient_color(position: f32, theme: &Theme) -> Color {
    if position <= 0.5 {
        theme::blend(theme.success, theme.warning, position * 2.0)
    } else {
        theme::blend(theme.warning, theme.danger, (position - 0.5) * 2.0)
    }
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
        spans.push(Span::styled(
            glyphs.bar_full,
            Style::default().fg(gradient_color(position, theme)),
        ));
    }
    if filled < width {
        spans.push(Span::styled(
            glyphs.bar_empty.repeat(width - filled),
            Style::default().fg(track_color(theme)),
        ));
    }
    spans
}

/// A sub-cell precision meter: the gradient bar with eighth-block partial
/// fills, so a 62.5% reading lights six cells and a half instead of six.
///
/// This is the polished face of [`gradient_bar`]; in ASCII mode it *is* the
/// gradient bar, which has no partial cells to draw with.
pub fn meter(ratio: f64, width: usize, theme: &Theme, unicode: bool) -> Vec<Span<'static>> {
    if !unicode || width == 0 {
        return gradient_bar(ratio, width, theme, unicode);
    }
    let glyphs = glyphs(unicode);
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };

    let eighths = (ratio * (width * 8) as f64).round() as usize;
    let full = (eighths / 8).min(width);
    let rem = eighths % 8;
    let partial = rem > 0 && full < width;

    let position = |cell: usize| {
        if width <= 1 {
            0.0
        } else {
            cell as f32 / (width - 1) as f32
        }
    };

    let mut spans = Vec::with_capacity(width + 1);
    for cell in 0..full {
        spans.push(Span::styled(
            glyphs.bar_full,
            Style::default().fg(gradient_color(position(cell), theme)),
        ));
    }
    if partial {
        spans.push(Span::styled(
            PARTIALS[rem - 1],
            Style::default().fg(gradient_color(position(full), theme)),
        ));
    }
    let used = full + usize::from(partial);
    if used < width {
        spans.push(Span::styled(
            glyphs.bar_empty.repeat(width - used),
            Style::default().fg(track_color(theme)),
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
/// The bar is the sub-cell gradient meter; `color` drives the value's text,
/// which is where the caller's severity judgement lives.
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
    spans.extend(meter(ratio, bar_width, theme, unicode));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        value.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    Line::from(spans)
}

/// A reversed pill: ` text ` on a coloured background, with the foreground
/// picked for contrast by the theme. Badges are how the UI says *state* —
/// `LIVE`, `REC`, `PAUSED` — at a glance.
///
/// A flat theme paints no backgrounds, so the pill degrades to bracketed
/// coloured text rather than reversing a block onto the user's terminal.
pub fn badge(text: &str, color: Color, theme: &Theme) -> Vec<Span<'static>> {
    if theme.flat() {
        return vec![Span::styled(
            format!("[{text}]"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )];
    }
    vec![Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(theme.on_color(color))
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )]
}

/// A keyboard-key chip, as shown in the status bar's hints: ` space ` on a
/// raised surface, like a keycap.
pub fn kbd(key: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![Span::styled(
        format!(" {key} "),
        Style::default()
            .fg(theme.accent)
            .bg(theme.surface_hi())
            .add_modifier(Modifier::BOLD),
    )]
}

/// A pill-shaped status chip, e.g. `● LIVE`.
///
/// Active chips are full badges; inactive ones dim toward the background, so
/// a blinking caller (the header's LIVE pulse) alternates between loud and
/// quiet rather than between bold and regular.
pub fn chip(
    text: &str,
    color: Color,
    active: bool,
    theme: &Theme,
    unicode: bool,
) -> Vec<Span<'static>> {
    let glyphs = glyphs(unicode);
    if active {
        badge(&format!("{} {text}", glyphs.dot), color, theme)
    } else {
        let dim = theme.fade(color, 0.45);
        vec![
            Span::styled(format!("{} ", glyphs.dot), Style::default().fg(dim)),
            Span::styled(text.to_string(), Style::default().fg(dim)),
        ]
    }
}

/// A labelled divider: `─ Title ─────────────────`, for separating groups
/// inside a panel without buying another border. Falls back to the bare
/// title when the row is too narrow to hold the rules.
pub fn section(title: &str, width: usize, theme: &Theme, unicode: bool) -> Line<'static> {
    let rule = if unicode { "─" } else { "-" };
    let fixed = 2 + title.chars().count() + 1; // rule + space, title, space
    if width <= fixed + 1 {
        return Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.accent_alt)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(vec![
        Span::styled(format!("{rule} "), Style::default().fg(theme.border)),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.accent_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", rule.repeat(width - fixed)),
            Style::default().fg(theme.border),
        ),
    ])
}

/// A full-width rule flowing along the brand gradient — the header's
/// underline, and the strongest single "this is a designed tool" signal on
/// screen.
pub fn gradient_rule(width: usize, theme: &Theme, unicode: bool) -> Vec<Span<'static>> {
    let glyph = if unicode { "─" } else { "-" };
    if width == 0 {
        return Vec::new();
    }
    (0..width)
        .map(|cell| {
            let t = cell as f32 / (width - 1).max(1) as f32;
            Span::styled(glyph, Style::default().fg(theme.gradient(t)))
        })
        .collect()
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
        assert!(meter(0.5, 0, theme(), true).is_empty());
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
    fn empty_cells_sit_on_the_track_token() {
        let spans = bar(0.0, 8, Color::Red, theme(), true);
        assert_eq!(spans[0].style.fg, Some(theme().track()));
    }

    #[test]
    fn a_meter_matches_the_gradient_bar_in_ascii_mode() {
        let metered = meter(0.5, 12, theme(), false);
        let graded = gradient_bar(0.5, 12, theme(), false);
        assert_eq!(width(&metered), width(&graded));
        let meter_text: String = metered.iter().map(|s| s.content.as_ref()).collect();
        let graded_text: String = graded.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(meter_text, graded_text);
    }

    #[test]
    fn a_meter_always_occupies_its_full_width() {
        for ratio in [0.0, 0.01, 0.13, 0.5, 0.94, 1.0] {
            let spans = meter(ratio, 16, theme(), true);
            assert_eq!(width(&spans), 16, "ratio {ratio} produced the wrong width");
        }
    }

    #[test]
    fn a_meter_fills_partial_cells_to_eighth_precision() {
        // Half a cell beyond four full ones: 4.5 / 10 cells.
        let spans = meter(0.45, 10, theme(), true);
        assert!(
            spans.iter().any(|span| span.content.as_ref() == "▌"),
            "no half-block partial in {spans:?}"
        );
        // A hair above zero still shows something.
        let hair = meter(0.01, 10, theme(), true);
        assert!(
            hair.iter().any(|span| span.content.as_ref() == "▏"),
            "a tiny ratio deserves a sliver: {hair:?}"
        );
    }

    #[test]
    fn a_meter_of_one_cell_does_not_divide_by_zero() {
        let spans = meter(1.0, 1, theme(), true);
        assert_eq!(width(&spans), 1);
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
    fn badges_reverse_onto_their_colour_with_a_contrasting_foreground() {
        let spans = badge("LIVE", theme().danger, theme());
        assert_eq!(spans[0].content, " LIVE ");
        assert_eq!(spans[0].style.bg, Some(theme().danger));
        assert_eq!(spans[0].style.fg, Some(theme().on_color(theme().danger)));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn badges_on_a_flat_theme_use_brackets_instead_of_a_fill() {
        let mono = theme::find("mono").unwrap();
        let spans = badge("LIVE", mono.danger, mono);
        assert_eq!(spans[0].content, "[LIVE]");
        assert_eq!(spans[0].style.bg, None);
        assert_eq!(spans[0].style.fg, Some(mono.danger));
    }

    #[test]
    fn key_chips_sit_on_a_raised_surface() {
        let spans = kbd("space", theme());
        assert_eq!(spans[0].content, " space ");
        assert_eq!(spans[0].style.bg, Some(theme().surface_hi()));
        assert_eq!(spans[0].style.fg, Some(theme().accent));
    }

    #[test]
    fn chips_are_badges_when_active_and_dimmed_text_when_not() {
        let active = chip("LIVE", Color::Red, true, theme(), true);
        assert_eq!(active[0].content, " ● LIVE ");
        assert_eq!(active[0].style.bg, Some(Color::Red));

        let idle = chip("LIVE", Color::Red, false, theme(), true);
        assert_eq!(idle[0].content, "● ");
        assert_eq!(idle[0].style.bg, None);

        assert_eq!(
            chip("LIVE", Color::Red, false, theme(), false)[0].content,
            "* "
        );
    }

    #[test]
    fn sections_span_their_width_and_degrade_to_a_bare_title() {
        let line = section("Keys", 30, theme(), true);
        assert_eq!(line.width(), 30);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("─ Keys "));

        let narrow = section("Keys", 6, theme(), true);
        assert_eq!(narrow.spans[0].content, "Keys");

        let ascii = section("Keys", 30, theme(), false);
        assert!(ascii.spans.iter().all(|s| s.content.is_ascii()));
    }

    #[test]
    fn gradient_rules_flow_from_accent_to_accent_alt() {
        let spans = gradient_rule(11, theme(), true);
        assert_eq!(spans.len(), 11);
        assert_eq!(spans[0].style.fg, Some(theme().accent));
        assert_eq!(spans[10].style.fg, Some(theme().accent_alt));
        assert!(gradient_rule(0, theme(), true).is_empty());
        assert!(
            gradient_rule(5, theme(), false)
                .iter()
                .all(|s| s.content == "-")
        );
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

    #[test]
    fn panels_are_filled_with_the_surface_token() {
        let block = panel("Test", false, theme(), true);
        let debug = format!("{block:?}");
        assert!(
            debug.contains(&format!("{:?}", theme().surface())),
            "panel does not paint the surface: {debug}"
        );
    }

    #[test]
    fn frame_panel_hands_back_exactly_the_area_inside_the_border() {
        let area = Rect::new(0, 0, 40, 10);
        let block = panel("System", false, theme::FALLBACK, true);
        let expected = block.inner(area);

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut got = Rect::default();
        terminal
            .draw(|frame| {
                got = frame_panel(frame, area, panel("System", false, theme::FALLBACK, true));
            })
            .unwrap();

        assert_eq!(got, expected, "the returned rect must be Block::inner");
        assert!(got.width < area.width && got.height < area.height);
    }
}

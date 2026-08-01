//! The six dashboards, plus the drawing helpers they share.

pub mod audio;
pub mod log;
pub mod overview;
pub mod performance;
pub mod scenes;
pub mod stream;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Chart, Dataset, GraphType, Paragraph, Sparkline};

use crate::metrics::History;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::widgets::chrome;

/// Below this width the three-column decks fold into a single column.
pub const NARROW_WIDTH: u16 = 96;

/// Draw the body of whichever view is active.
pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    match app.view {
        View::Overview => overview::render(frame, area, app),
        View::Performance => performance::render(frame, area, app),
        View::Stream => stream::render(frame, area, app),
        View::Scenes => scenes::render(frame, area, app),
        View::Audio => audio::render(frame, area, app),
        View::Log => log::render(frame, area, app),
    }
}

/// Fill `area` with a sparkline of the newest samples from `history`.
///
/// Sparkline scales to the maximum in the data it is given, so a flat series
/// would otherwise render as a full bar; `floor` sets a minimum ceiling to
/// keep quiet periods looking quiet.
pub fn sparkline(
    frame: &mut Frame,
    area: Rect,
    history: &History,
    floor: f64,
    color: Color,
    unicode: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Sparkline scales bars against its own max, so subtract the windowed
    // floor first: otherwise a 59-60fps series renders as a solid block.
    let (base, top) = history.axis_range(floor);
    let span = (top - base).max(1.0);
    let normalized: Vec<f64> = history
        .tail(area.width as usize)
        .into_iter()
        .map(|sample| ((sample - base) / span).clamp(0.0, 1.0))
        .collect();

    if !unicode {
        // ratatui's Sparkline is built from U+2580 block elements and has no
        // ASCII mode, so the fallback draws its own bars.
        frame.render_widget(
            Paragraph::new(ascii_bars(&normalized, area.width, area.height, color)),
            area,
        );
        return;
    }

    let data: Vec<u64> = normalized
        .into_iter()
        .map(|value| (value * 100.0).round() as u64)
        .collect();

    frame.render_widget(
        Sparkline::default()
            .data(&data)
            .max(100)
            .style(Style::default().fg(color)),
        area,
    );
}

/// A column chart of `values` (each `0.0..=1.0`) drawn with `#`.
///
/// Rows are produced top-down, each column filled from the bottom up, which
/// is the same reading as the Unicode sparkline it stands in for.
fn ascii_bars(values: &[f64], width: u16, height: u16, color: Color) -> Vec<Line<'static>> {
    let style = Style::default().fg(color);
    let width = width as usize;
    let height = height as usize;
    // Newest sample on the right: a partly-filled history leaves blank columns
    // on the left rather than stretching to fill the area.
    let offset = width.saturating_sub(values.len());

    (0..height)
        .map(|row| {
            // Row 0 is the top, so it stands for the highest band.
            let threshold = (height - row - 1) as f64 / height as f64;
            let text: String = (0..width)
                .map(|column| {
                    match column
                        .checked_sub(offset)
                        .and_then(|index| values.get(index))
                    {
                        Some(value) if *value > threshold => '#',
                        _ => ' ',
                    }
                })
                .collect();
            Line::from(Span::styled(text, style))
        })
        .collect()
}

/// A braille line chart of `history` over `area`, labelled on both axes.
///
/// `fmt` renders the y-axis labels — raw numbers are unreadable for bitrates,
/// so each caller supplies the units it thinks in. `floor` sets the minimum
/// top of the axis so an idle metric is not magnified into a mountain range.
///
/// The parameter list is long because a chart genuinely has this many knobs;
/// wrapping them in a builder would add indirection without removing any.
#[allow(clippy::too_many_arguments)]
pub fn line_chart(
    frame: &mut Frame,
    area: Rect,
    history: &History,
    label: &str,
    fmt: fn(f64) -> String,
    floor: f64,
    color: Color,
    theme: &Theme,
    unicode: bool,
) {
    if area.width < 8 || area.height < 4 {
        // Too small for axes; degrade to a sparkline rather than drawing a
        // chart whose labels would eat the whole panel.
        sparkline(frame, area, history, floor, color, unicode);
        return;
    }

    let points = history.tail_points(area.width as usize);
    if points.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder("waiting for data…", theme)),
            area,
        );
        return;
    }

    let x_max = (points.len().saturating_sub(1)).max(1) as f64;
    let (y_min, y_max) = history.axis_range(floor);

    if !unicode {
        // Chart draws its axes with box-drawing runes and its series with
        // braille, neither of which has an ASCII substitute in ratatui.
        frame.render_widget(
            Paragraph::new(ascii_plot(history, area, y_min, y_max, fmt, color, theme)),
            area,
        );
        return;
    }

    let dataset = Dataset::default()
        .name(label.to_string())
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&points);

    let axis_style = Style::default().fg(theme.muted);
    let chart = Chart::new(vec![dataset])
        // Chart paints its plot area through a Canvas whose background is
        // `self.style.bg.unwrap_or(Color::Reset)`. Leave the style unset and
        // that Reset punches a hole straight through the themed background,
        // exposing the terminal's own — visible as a mismatched rectangle
        // (and any wallpaper or transparency behind it). The theme's own
        // `Color::Reset`, as in the mono palette, still passes through here,
        // which is what that theme wants.
        .style(Style::default().bg(theme.bg))
        .x_axis(
            Axis::default()
                .style(Style::default().fg(theme.border))
                .bounds([0.0, x_max]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(theme.border))
                .labels(vec![
                    Span::styled(fmt(y_min), axis_style),
                    Span::styled(fmt((y_min + y_max) / 2.0), axis_style),
                    Span::styled(fmt(y_max), axis_style),
                ])
                .bounds([y_min, y_max]),
        )
        .legend_position(None);

    frame.render_widget(chart, area);
}

/// An ASCII scatter plot with a labelled left gutter.
///
/// Stands in for [`ratatui::widgets::Chart`], which has no ASCII mode: its
/// axes are box-drawing runes and its series is braille.
fn ascii_plot(
    history: &History,
    area: Rect,
    y_min: f64,
    y_max: f64,
    fmt: fn(f64) -> String,
    color: Color,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let gutter = fmt(y_max).chars().count().max(fmt(y_min).chars().count()) + 1;
    let plot_width = (area.width as usize).saturating_sub(gutter + 1);
    let height = area.height as usize;
    if plot_width == 0 || height == 0 {
        return Vec::new();
    }

    let samples = history.tail(plot_width);
    // Newest sample on the right; a short history leaves the left blank.
    let offset = plot_width.saturating_sub(samples.len());
    let span = (y_max - y_min).max(f64::EPSILON);
    let axis_style = Style::default().fg(theme.muted);
    let border_style = Style::default().fg(theme.border);

    (0..height)
        .map(|row| {
            // Label only the extremes and the midpoint, as the Unicode chart does.
            let label = match row {
                0 => fmt(y_max),
                _ if row == height / 2 => fmt((y_min + y_max) / 2.0),
                _ if row == height - 1 => fmt(y_min),
                _ => String::new(),
            };

            let text: String = (0..plot_width)
                .map(|column| {
                    let Some(sample) = column
                        .checked_sub(offset)
                        .and_then(|index| samples.get(index))
                    else {
                        return ' ';
                    };
                    let fraction = ((sample - y_min) / span).clamp(0.0, 1.0);
                    // Row 0 is the top, so invert the fraction to a row index.
                    let plotted = ((1.0 - fraction) * (height - 1) as f64).round() as usize;
                    if plotted == row { '*' } else { ' ' }
                })
                .collect();

            Line::from(vec![
                Span::styled(format!("{label:>gutter$}"), axis_style),
                Span::styled("|", border_style),
                Span::styled(text, Style::default().fg(color)),
            ])
        })
        .collect()
}

/// A large centred verdict word, e.g. `HEALTHY`.
pub fn verdict(text: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
    .centered()
}

/// Split a metric's value and unit so the unit can be dimmed.
pub fn value_unit(
    value: impl Into<String>,
    unit: &str,
    color: Color,
    theme: &Theme,
) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            value.into(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {unit}"), Style::default().fg(theme.muted)),
    ]
}

/// Y-axis label for a plain count, e.g. FPS.
pub fn axis_count(value: f64) -> String {
    format!("{value:.0}")
}

/// Y-axis label for a percentage.
pub fn axis_percent(value: f64) -> String {
    format!("{value:.0}%")
}

/// Y-axis label for a bit rate, in the units streamers read.
pub fn axis_bitrate(value: f64) -> String {
    crate::util::bitrate(value)
}

/// Y-axis label for a duration in milliseconds.
pub fn axis_millis(value: f64) -> String {
    format!("{value:.1}ms")
}

/// Width available for a bar in a panel `total` columns wide, given a label
/// column and the space the value needs on the right.
///
/// Returns 0 when the panel is too narrow for a bar at all, which every
/// caller treats as "just print the number".
pub fn bar_width(total: u16, label_width: usize, value_width: usize) -> usize {
    // 2 for the panel border, 2 for its horizontal padding, 2 for the gap
    // between the bar and the value.
    let overhead = 6 + label_width + value_width;
    (total as usize).saturating_sub(overhead).min(40)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_width_leaves_room_for_the_label_value_and_chrome() {
        // 80 columns, 8-wide label, 8-wide value → 80 - 6 - 8 - 8 = 58, capped.
        assert_eq!(bar_width(80, 8, 8), 40);
        assert_eq!(bar_width(40, 8, 8), 18);
    }

    #[test]
    fn bar_width_collapses_to_zero_in_a_narrow_panel() {
        assert_eq!(bar_width(20, 8, 8), 0);
        assert_eq!(bar_width(0, 8, 8), 0);
        assert_eq!(bar_width(22, 8, 8), 0);
        assert_eq!(bar_width(24, 8, 8), 2);
    }

    #[test]
    fn bar_width_is_capped_so_wide_terminals_stay_readable() {
        assert_eq!(bar_width(500, 8, 8), 40);
    }

    fn grid(rows: &[Line<'_>]) -> Vec<String> {
        rows.iter()
            .map(|line| line.spans[0].content.to_string())
            .collect()
    }

    #[test]
    fn ascii_bars_fill_from_the_bottom_up() {
        // Column 0 full, column 1 half, column 2 empty.
        let rows = grid(&ascii_bars(&[1.0, 0.5, 0.0], 3, 4, Color::Green));
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0], "#  ",
            "only the tallest column reaches the top row"
        );
        assert_eq!(rows[3], "## ", "the bottom row holds every non-zero column");
    }

    #[test]
    fn ascii_bars_right_align_a_partly_filled_history() {
        // Two samples in a six-wide area belong on the right, where the
        // newest data is — not stretched or left-aligned.
        let rows = grid(&ascii_bars(&[1.0, 1.0], 6, 1, Color::Green));
        assert_eq!(rows[0], "    ##");
    }

    #[test]
    fn ascii_bars_survive_a_degenerate_area() {
        assert!(
            ascii_bars(&[1.0], 0, 1, Color::Green)[0].spans[0]
                .content
                .is_empty()
        );
        assert!(ascii_bars(&[1.0], 4, 0, Color::Green).is_empty());
        assert!(
            ascii_bars(&[], 4, 2, Color::Green)
                .iter()
                .all(|line| { line.spans[0].content.chars().all(|c| c == ' ') })
        );
    }

    #[test]
    fn ascii_bars_are_pure_ascii() {
        for row in grid(&ascii_bars(&[0.0, 0.5, 1.0], 8, 3, Color::Green)) {
            assert!(row.is_ascii(), "{row:?} is not ASCII");
        }
    }

    #[test]
    fn axis_labels_carry_their_units() {
        assert_eq!(axis_count(59.94), "60");
        assert_eq!(axis_percent(4.2), "4%");
        assert_eq!(axis_bitrate(6_000_000.0), "6.00 Mbps");
        assert_eq!(axis_millis(16.67), "16.7ms");
    }

    #[test]
    fn verdict_is_centred_and_coloured() {
        let line = verdict("HEALTHY", Color::Green);
        assert_eq!(line.spans[0].content, "HEALTHY");
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn value_and_unit_are_styled_separately() {
        let spans = value_unit("59.9", "fps", Color::Green, crate::tui::theme::FALLBACK);
        assert_eq!(spans[0].content, "59.9");
        assert_eq!(spans[1].content, " fps");
        assert_ne!(spans[0].style.fg, spans[1].style.fg);
    }
}

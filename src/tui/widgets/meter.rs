//! Segmented audio peak meters.
//!
//! Levels arrive from OBS as linear magnitudes, but ears and mixing desks work
//! in dB, so the bar is mapped on a dB scale: normal speech around -20 dBFS
//! fills roughly two thirds of the meter instead of the ~10% a linear scale
//! would show.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::obs::stats::AudioInput;
use crate::tui::theme::{self, Theme};
use crate::tui::widgets::chrome;
use crate::util;

/// Bottom of the meter's scale, in dBFS.
pub const FLOOR_DB: f32 = -60.0;

/// Fraction of the meter above which the segment turns amber, then red.
/// -9 dBFS and -3 dBFS respectively on the default 60 dB scale.
const WARN_AT: f32 = 0.85;
const DANGER_AT: f32 = 0.95;

/// A meter bar of `width` cells for a linear peak level.
pub fn bar(
    level: f32,
    muted: bool,
    width: usize,
    theme: &Theme,
    unicode: bool,
) -> Vec<Span<'static>> {
    let glyphs = chrome::glyphs(unicode);
    let filled = if muted { 0 } else { fill_cells(level, width) };

    let mut spans = Vec::with_capacity(width);
    for cell in 0..width {
        if cell < filled {
            let position = if width <= 1 {
                0.0
            } else {
                cell as f32 / (width - 1) as f32
            };
            spans.push(Span::styled(
                glyphs.meter_full.to_string(),
                Style::default().fg(segment_color(position, theme)),
            ));
        } else {
            spans.push(Span::styled(
                glyphs.meter_empty.to_string(),
                Style::default().fg(chrome::track_color(theme)),
            ));
        }
    }
    spans
}

/// A full meter row: name, fader position, bar, and the peak in dB.
pub fn row(
    input: &AudioInput,
    name_width: usize,
    bar_width: usize,
    theme: &Theme,
    unicode: bool,
) -> Line<'static> {
    let AudioInput {
        name,
        muted,
        volume_db,
        ..
    } = input;
    let (muted, volume_db) = (*muted, *volume_db);
    let peak = input.peak();

    let glyphs = chrome::glyphs(unicode);
    let label_color = if muted { theme.muted } else { theme.fg };

    let mut spans = vec![
        Span::styled(
            // Single-column marker: an emoji here would push every following
            // column right on this row alone and break the table alignment.
            format!("{} ", muted_marker(muted, unicode, glyphs.bullet)),
            Style::default().fg(if muted { theme.danger } else { theme.success }),
        ),
        Span::styled(
            format!(
                "{:<width$}",
                util::truncate(name, name_width),
                width = name_width
            ),
            Style::default().fg(label_color).add_modifier(if muted {
                Modifier::DIM
            } else {
                Modifier::empty()
            }),
        ),
        Span::styled(
            format!("{:>8}  ", fader_label(muted, volume_db)),
            Style::default().fg(theme.muted),
        ),
    ];

    spans.extend(bar(peak, muted, bar_width, theme, unicode));
    spans.push(Span::styled(
        format!("  {:>6}", peak_label(muted, peak)),
        Style::default().fg(peak_color(muted, peak, theme)),
    ));
    Line::from(spans)
}

/// Row marker for a muted / unmuted input, always one column wide.
fn muted_marker(muted: bool, unicode: bool, bullet: &'static str) -> &'static str {
    match (muted, unicode) {
        (true, true) => "✕",
        (true, false) => "x",
        (false, _) => bullet,
    }
}

/// How many cells a level fills, on the dB scale.
pub fn fill_cells(level: f32, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let db = util::linear_to_db(level);
    let fraction = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    ((fraction * width as f32).round() as usize).min(width)
}

/// Colour of a segment at `position` along the bar.
fn segment_color(position: f32, theme: &Theme) -> ratatui::style::Color {
    if position >= DANGER_AT {
        theme.danger
    } else if position >= WARN_AT {
        theme.warning
    } else {
        // Blend across the healthy stretch so the meter reads as a gradient
        // rather than three flat blocks.
        theme::blend(theme.success, theme.info, position / WARN_AT)
    }
}

fn fader_label(muted: bool, volume_db: f64) -> String {
    if muted {
        "muted".to_string()
    } else if !volume_db.is_finite() || volume_db <= -100.0 {
        "-inf dB".to_string()
    } else {
        format!("{volume_db:+.1} dB")
    }
}

fn peak_label(muted: bool, peak: f32) -> String {
    if muted || peak <= 0.0 {
        "—".to_string()
    } else {
        format!("{:.0}", util::linear_to_db(peak))
    }
}

fn peak_color(muted: bool, peak: f32, theme: &Theme) -> ratatui::style::Color {
    if muted {
        return theme.muted;
    }
    let db = util::linear_to_db(peak);
    // Clipping is at 0 dBFS; -6 dB is the usual "you are getting hot" mark.
    if db >= -1.0 {
        theme.danger
    } else if db >= -6.0 {
        theme.warning
    } else {
        theme.fg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> &'static Theme {
        theme::FALLBACK
    }

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn a_meter_always_occupies_its_full_width() {
        for level in [0.0, 0.001, 0.1, 0.5, 1.0] {
            let spans = bar(level, false, 20, theme(), true);
            assert_eq!(spans.len(), 20, "level {level} produced the wrong width");
        }
    }

    #[test]
    fn silence_fills_nothing_and_full_scale_fills_everything() {
        assert_eq!(fill_cells(0.0, 20), 0);
        assert_eq!(fill_cells(1.0, 20), 20);
    }

    #[test]
    fn the_scale_is_logarithmic_not_linear() {
        // -20 dBFS is 0.1 linear. On a linear scale that would be 2 cells of
        // 20; on the dB scale it should be about two thirds of the meter.
        let cells = fill_cells(0.1, 30);
        assert!(
            (19..=21).contains(&cells),
            "expected ~20 of 30 cells for -20 dBFS, got {cells}"
        );
    }

    #[test]
    fn levels_below_the_floor_read_as_silence() {
        assert_eq!(fill_cells(1e-9, 20), 0);
        assert_eq!(fill_cells(0.0009, 20), 0, "-60 dBFS is the floor");
    }

    #[test]
    fn a_muted_input_shows_an_empty_meter_whatever_the_level() {
        let spans = bar(1.0, true, 10, theme(), true);
        assert_eq!(spans.len(), 10);
        assert!(
            spans.iter().all(|span| span.content == "▱"),
            "a muted input must not show level"
        );
    }

    #[test]
    fn a_zero_width_meter_is_empty_rather_than_panicking() {
        assert!(bar(0.5, false, 0, theme(), true).is_empty());
        assert_eq!(fill_cells(0.5, 0), 0);
    }

    #[test]
    fn hot_segments_turn_amber_then_red() {
        assert_eq!(segment_color(0.0, theme()), theme().success);
        assert_eq!(segment_color(0.9, theme()), theme().warning);
        assert_eq!(segment_color(1.0, theme()), theme().danger);
    }

    #[test]
    fn fader_labels_cover_muted_unity_and_negative_infinity() {
        assert_eq!(fader_label(true, 0.0), "muted");
        assert_eq!(fader_label(false, 0.0), "+0.0 dB");
        assert_eq!(fader_label(false, -6.5), "-6.5 dB");
        assert_eq!(fader_label(false, f64::NEG_INFINITY), "-inf dB");
        assert_eq!(fader_label(false, -100.0), "-inf dB");
    }

    #[test]
    fn peak_labels_show_a_dash_for_silence() {
        assert_eq!(peak_label(false, 0.0), "—");
        assert_eq!(peak_label(true, 0.9), "—");
        assert_eq!(peak_label(false, 1.0), "0");
        assert_eq!(peak_label(false, 0.1), "-20");
    }

    #[test]
    fn peak_colour_warns_before_it_clips() {
        assert_eq!(peak_color(false, 0.01, theme()), theme().fg);
        // -3 dBFS ≈ 0.7 linear: hot but not clipping.
        assert_eq!(peak_color(false, 0.7, theme()), theme().warning);
        assert_eq!(peak_color(false, 1.0, theme()), theme().danger);
        assert_eq!(peak_color(true, 1.0, theme()), theme().muted);
    }

    fn input(name: &str, muted: bool, peak: f32) -> AudioInput {
        AudioInput {
            name: name.to_string(),
            kind: "pulse_input_capture".to_string(),
            muted,
            volume_db: -6.0,
            volume_mul: 0.5,
            peaks: vec![peak],
        }
    }

    #[test]
    fn a_row_contains_the_name_fader_and_peak() {
        let line = row(&input("Mic/Aux", false, 0.1), 12, 16, theme(), true);
        let rendered = text(&line.spans);
        assert!(rendered.contains("Mic/Aux"));
        assert!(rendered.contains("-6.0 dB"));
        assert!(rendered.contains("-20"));
    }

    #[test]
    fn a_long_input_name_is_truncated_to_its_column() {
        let line = row(
            &input("An Extremely Long Source Name", false, 0.0),
            12,
            16,
            theme(),
            true,
        );
        assert!(text(&line.spans).contains('…'));
    }

    #[test]
    fn the_muted_marker_is_always_one_column_wide() {
        for muted in [true, false] {
            for unicode in [true, false] {
                let marker = muted_marker(muted, unicode, "•");
                assert_eq!(marker.chars().count(), 1, "{marker} is not one column");
                assert!((marker.chars().next().unwrap() as u32) < 0x1F000);
            }
        }
    }

    #[test]
    fn muted_and_unmuted_rows_start_at_the_same_column() {
        let loud = row(&input("Mic", false, 0.5), 12, 16, theme(), true);
        let quiet = row(&input("Mic", true, 0.5), 12, 16, theme(), true);
        assert_eq!(
            loud.spans[0].content.chars().count(),
            quiet.spans[0].content.chars().count(),
            "the marker column must not change width with mute state"
        );
    }

    #[test]
    fn a_muted_row_says_muted_rather_than_showing_a_fader() {
        let line = row(&input("Mic", true, 0.9), 12, 16, theme(), true);
        let rendered = text(&line.spans);
        assert!(rendered.contains("muted"));
        assert!(!rendered.contains("-6.0 dB"));
    }
}

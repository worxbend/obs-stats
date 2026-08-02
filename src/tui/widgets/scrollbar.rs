//! A one-column scrollbar for the list views.
//!
//! ratatui ships a `Scrollbar`, but its track, thumb and end caps are box
//! drawing, block elements and arrows — the three ranges `--ascii` exists to
//! avoid, and it has no ASCII mode. So this draws its own, the same way
//! `views::ascii_plot` stands in for `Chart`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::theme::Theme;

/// Where a scrollbar's thumb sits, in rows from the top of the track.
///
/// Split out from drawing so the arithmetic — which is what a drag maps
/// through — is testable on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thumb {
    pub start: u16,
    pub height: u16,
}

/// Size and place the thumb for a list of `total` rows showing `visible` of
/// them from `offset`.
///
/// Returns `None` when everything fits, which is when a scrollbar would be
/// decoration rather than information.
pub fn thumb(track: u16, total: usize, visible: usize, offset: usize) -> Option<Thumb> {
    if track == 0 || total == 0 || visible == 0 || total <= visible {
        return None;
    }

    // At least one row, so the thumb never vanishes on a very long list.
    let height = ((visible as f64 / total as f64) * track as f64)
        .round()
        .max(1.0) as u16;
    let height = height.min(track);

    // The last scroll position must put the thumb flush with the bottom, or
    // the bar implies there is more below when there is not.
    let scrollable_rows = (total - visible) as f64;
    let travel = track.saturating_sub(height) as f64;
    let start = ((offset.min(total - visible) as f64 / scrollable_rows) * travel).round() as u16;

    Some(Thumb {
        start: start.min(track - height),
        height,
    })
}

/// Which row of a `total`-row list a drag to `row` within `track` means.
///
/// The inverse of [`thumb`], near enough: grabbing the middle of the bar puts
/// you in the middle of the list.
pub fn row_at(track: u16, total: usize, row: u16) -> usize {
    if track <= 1 || total == 0 {
        return 0;
    }
    let fraction = f64::from(row.min(track - 1)) / f64::from(track - 1);
    ((fraction * (total - 1) as f64).round() as usize).min(total - 1)
}

/// Draw the bar down `area`, which should be one column wide.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    offset: usize,
    theme: &Theme,
    unicode: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(thumb) = thumb(area.height, total, area.height as usize, offset) else {
        return;
    };

    let (track_glyph, thumb_glyph) = if unicode { ("│", "█") } else { ("|", "#") };
    let track_style = Style::default().fg(theme.border);
    let thumb_style = Style::default().fg(theme.accent);

    let lines: Vec<Line> = (0..area.height)
        .map(|row| {
            let on_thumb = row >= thumb.start && row < thumb.start + thumb.height;
            if on_thumb {
                Line::from(Span::styled(thumb_glyph, thumb_style))
            } else {
                Line::from(Span::styled(track_glyph, track_style))
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_that_fits_gets_no_scrollbar() {
        assert_eq!(thumb(10, 5, 10, 0), None);
        assert_eq!(thumb(10, 10, 10, 0), None, "exactly full still fits");
        assert_eq!(thumb(0, 100, 10, 0), None, "no track");
        assert_eq!(thumb(10, 0, 10, 0), None, "no rows");
    }

    #[test]
    fn the_thumb_is_proportional_to_how_much_is_visible() {
        // A quarter of the list on screen fills a quarter of the track.
        let thumb = thumb(20, 40, 10, 0).unwrap();
        assert_eq!(thumb.height, 5);
        assert_eq!(thumb.start, 0);
    }

    #[test]
    fn the_thumb_never_shrinks_away_on_a_very_long_list() {
        let thumb = thumb(10, 100_000, 10, 0).unwrap();
        assert_eq!(thumb.height, 1, "still visible, still honest");
    }

    #[test]
    fn scrolling_to_the_end_puts_the_thumb_flush_with_the_bottom() {
        let track = 20;
        let thumb = thumb(track, 100, 10, 90).unwrap();
        assert_eq!(
            thumb.start + thumb.height,
            track,
            "a gap below would imply there is more to see"
        );
    }

    #[test]
    fn the_thumb_stays_inside_the_track_however_odd_the_offset() {
        for offset in [0, 1, 45, 89, 90, 500, usize::MAX / 2] {
            let thumb = thumb(20, 100, 10, offset).unwrap();
            assert!(
                thumb.start + thumb.height <= 20,
                "offset {offset} pushed the thumb out of the track"
            );
        }
    }

    #[test]
    fn a_drag_maps_the_track_onto_the_list() {
        assert_eq!(row_at(10, 100, 0), 0);
        assert_eq!(row_at(10, 100, 9), 99, "the bottom of the track is the end");
        assert_eq!(row_at(11, 101, 5), 50, "the middle is the middle");
    }

    #[test]
    fn a_drag_past_the_track_clamps_rather_than_overshooting() {
        assert_eq!(row_at(10, 100, 200), 99);
        assert_eq!(row_at(1, 100, 0), 0, "a one-row track has one answer");
        assert_eq!(row_at(10, 0, 5), 0, "an empty list has no rows to hit");
    }

    #[test]
    fn drawing_a_degenerate_area_is_a_no_op() {
        use crate::tui::theme;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(10, 10)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 0, 5), 100, 0, theme::FALLBACK, true);
                render(frame, Rect::new(0, 0, 1, 0), 100, 0, theme::FALLBACK, true);
            })
            .unwrap();
    }

    #[test]
    fn ascii_mode_avoids_box_drawing_and_block_glyphs() {
        use crate::tui::theme;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(1, 10)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    Rect::new(0, 0, 1, 10),
                    100,
                    20,
                    theme::FALLBACK,
                    false,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        for y in 0..10 {
            let symbol = buffer[(0, y)].symbol();
            assert!(symbol.is_ascii(), "{symbol:?} is not ASCII");
        }
    }

    #[test]
    fn the_drawn_thumb_is_where_the_arithmetic_says() {
        use crate::tui::theme;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(1, 20)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 1, 20), 40, 0, theme::FALLBACK, false);
            })
            .unwrap();

        let buffer = terminal.backend().buffer().clone();
        let drawn: String = (0..20)
            .map(|y| buffer[(0, y)].symbol().to_string())
            .collect();
        let expected = thumb(20, 40, 20, 0);
        // 20 of 40 rows visible: half the track, at the top.
        assert_eq!(
            expected,
            Some(Thumb {
                start: 0,
                height: 10
            })
        );
        assert_eq!(drawn, "##########||||||||||");
    }
}

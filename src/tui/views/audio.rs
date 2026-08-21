//! The Audio view — every audio-bearing input with its fader and peak meter.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::obs::stats::AudioInput;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::widgets::{chrome, meter};

/// Columns reserved for the input name.
const NAME_WIDTH: usize = 22;

/// Columns the marker, fader label and peak readout need around the meter.
const FIXED_WIDTH: usize = 2 + NAME_WIDTH + 10 + 8;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = app.theme;
    let inputs = app.snapshot.audio_inputs().to_vec();
    app.set_row_count(View::Audio, inputs.len());
    // Owned by the app so the settled offset outlives the frame; see
    // `App::take_list_state`.
    let mut state = app.take_list_state(View::Audio);

    let [list_area, legend_area] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(3)]).areas(area);

    let block = chrome::panel_with_note(
        "Audio inputs",
        format!("{}", inputs.len()),
        true,
        theme,
        app.unicode,
    );
    let inner = block.inner(list_area);
    frame.render_widget(block, list_area);

    if inputs.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder(
                "no audio inputs — OBS reports none, or it is not connected",
                theme,
            )),
            inner,
        );
    } else {
        let bar_width = meter_width(inner.width);
        let items: Vec<ListItem> = inputs
            .iter()
            .map(|input| ListItem::new(row(input, bar_width, theme, app.unicode)))
            .collect();

        let list = List::new(items).highlight_style(
            Style::default()
                .bg(theme.highlight_bg)
                .fg(theme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        );
        state.select(Some(app.selection(View::Audio)));
        frame.render_stateful_widget(list, inner, &mut state);
    }
    app.put_list_state(View::Audio, inner, state);

    let legend_block = chrome::panel("Scale", false, theme, app.unicode);
    let legend_inner = legend_block.inner(legend_area);
    frame.render_widget(legend_block, legend_area);
    frame.render_widget(
        Paragraph::new(legend(meter_width(legend_inner.width), theme, app.unicode)),
        legend_inner,
    );
}

/// How many cells the meter bar gets in a panel `width` columns wide.
///
/// Clamped so the meter neither vanishes on a narrow terminal nor stretches
/// to an unreadable width on a very wide one.
pub fn meter_width(width: u16) -> usize {
    (width as usize).saturating_sub(FIXED_WIDTH).clamp(8, 48)
}

/// One input's row.
pub fn row(input: &AudioInput, bar_width: usize, theme: &Theme, unicode: bool) -> Line<'static> {
    meter::row(input, NAME_WIDTH, bar_width, theme, unicode)
}

/// The dB scale under the meters, so the bar lengths mean something.
pub fn legend(bar_width: usize, theme: &Theme, unicode: bool) -> Vec<Line<'static>> {
    let glyphs = chrome::glyphs(unicode);
    let mut spans = vec![Span::styled(
        format!("{:<width$}", "-60 dBFS", width = 10),
        Style::default().fg(theme.muted),
    )];

    // Draw the scale in the same colours a live meter would use, so the
    // legend reads as a key rather than as another input.
    for cell in 0..bar_width {
        let position = if bar_width <= 1 {
            0.0
        } else {
            cell as f32 / (bar_width - 1) as f32
        };
        let color = if position >= 0.95 {
            theme.danger
        } else if position >= 0.85 {
            theme.warning
        } else {
            crate::tui::theme::blend(theme.success, theme.info, position / 0.85)
        };
        spans.push(Span::styled(
            glyphs.meter_full.to_string(),
            Style::default().fg(color),
        ));
    }

    spans.push(Span::styled("  0 dBFS", Style::default().fg(theme.muted)));

    vec![Line::from(spans)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::testing::line_text;
    use crate::tui::theme;

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
    fn meter_width_is_clamped_at_both_ends() {
        assert_eq!(meter_width(0), 8, "never vanishes");
        assert_eq!(meter_width(40), 8);
        assert_eq!(meter_width(1000), 48, "never stretches absurdly");
        assert_eq!(meter_width(72), 30);
    }

    #[test]
    fn a_row_shows_the_name_fader_and_a_meter() {
        let line = row(&input("Mic/Aux", false, 0.1), 20, theme::FALLBACK, true);
        let rendered = line_text(&line);
        assert!(rendered.contains("Mic/Aux"));
        assert!(rendered.contains("-6.0 dB"));
        assert!(rendered.contains('▰'), "the meter should show level");
    }

    #[test]
    fn a_muted_row_shows_no_level_however_loud_the_source() {
        let line = row(&input("Mic", true, 1.0), 20, theme::FALLBACK, true);
        let rendered = line_text(&line);
        assert!(rendered.contains("muted"));
        assert!(!rendered.contains('▰'), "a muted input must read as silent");
    }

    #[test]
    fn a_silent_row_shows_an_empty_meter() {
        let line = row(&input("Desktop", false, 0.0), 20, theme::FALLBACK, true);
        assert!(!line_text(&line).contains('▰'));
    }

    #[test]
    fn an_input_with_no_meter_data_yet_does_not_panic() {
        let mut quiet = input("Mic", false, 0.0);
        quiet.peaks.clear();
        let rendered = line_text(&row(&quiet, 20, theme::FALLBACK, true));
        assert!(rendered.contains("Mic"));
    }

    #[test]
    fn the_legend_spans_the_full_scale() {
        let lines = legend(20, theme::FALLBACK, true);
        let rendered = line_text(&lines[0]);
        assert!(rendered.contains("-60 dBFS"));
        assert!(rendered.contains("0 dBFS"));
        assert_eq!(rendered.matches('▰').count(), 20);
    }

    #[test]
    fn the_legend_runs_from_healthy_to_clipping() {
        let lines = legend(20, theme::FALLBACK, true);
        let cells = &lines[0].spans[1..21];
        assert_eq!(cells[0].style.fg, Some(theme::FALLBACK.success));
        assert_eq!(cells[19].style.fg, Some(theme::FALLBACK.danger));
    }

    #[test]
    fn the_legend_survives_a_single_cell_meter() {
        let lines = legend(1, theme::FALLBACK, true);
        assert_eq!(line_text(&lines[0]).matches('▰').count(), 1);
    }
}

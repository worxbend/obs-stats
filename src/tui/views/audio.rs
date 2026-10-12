//! The Audio view — every audio-bearing input with its fader and peak meter.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::obs::stats::AudioInput;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::view::View;
use crate::tui::views;
use crate::tui::widgets::list;
use crate::tui::widgets::{chrome, meter};
use crate::util;

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

    let [summary, body, legend_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .areas(area);
    render_summary(frame, summary, &inputs, app);
    let [list_area, inspection] = if area.width >= 110 {
        Layout::horizontal([Constraint::Min(50), Constraint::Length(34)]).areas(body)
    } else {
        [body, Rect::default()]
    };

    let block = chrome::panel_with_note(
        "Audio inputs",
        "fader / peak dBFS",
        true,
        theme,
        app.unicode,
    );
    let panel_inner = chrome::frame_panel(frame, list_area, block);
    let [header, inner] = Layout::vertical([
        Constraint::Length(u16::from(!inputs.is_empty())),
        Constraint::Min(0),
    ])
    .areas(panel_inner);
    let (name_width, bar_width) = row_dimensions(inner.width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if bar_width < 5 {
                "INPUT / FADER / PEAK".to_string()
            } else {
                format!(
                    "    {:<name_width$}{:>8}  {:<bar_width$}  {:>6}",
                    "INPUT", "FADER", "LEVEL", "PEAK"
                )
            },
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ))),
        header,
    );

    if inputs.is_empty() {
        frame.render_widget(
            Paragraph::new(chrome::placeholder(
                "no audio inputs — OBS reports none, or it is not connected",
                theme,
            )),
            inner,
        );
    } else {
        let items: Vec<Line> = inputs
            .iter()
            .map(|input| meter::row(input, name_width, bar_width, theme, app.unicode))
            .collect();

        list::render_rows(
            frame,
            inner,
            items,
            app.selection(View::Audio),
            &mut state,
            theme,
            app.unicode,
        );
    }
    app.put_list_state(View::Audio, inner, state);
    if inspection.width > 0 {
        let inner = chrome::frame_panel(
            frame,
            inspection,
            chrome::panel_with_note("Selected input", "read-only", false, theme, app.unicode),
        );
        frame.render_widget(
            Paragraph::new(input_detail(
                inputs.get(app.selection(View::Audio)),
                theme,
                app.unicode,
            ))
            .wrap(Wrap { trim: false }),
            inner,
        );
    }

    let legend_block = chrome::panel_with_note(
        "Scale",
        "peak, not loudness / latest meter event",
        false,
        theme,
        app.unicode,
    );
    let legend_inner = chrome::frame_panel(frame, legend_area, legend_block);
    frame.render_widget(
        Paragraph::new(legend(meter_width(legend_inner.width), theme, app.unicode)),
        legend_inner,
    );
}

fn render_summary(frame: &mut Frame, area: Rect, inputs: &[AudioInput], app: &App) {
    let regions = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(area);
    let muted = inputs.iter().filter(|input| input.muted).count();
    let metered = inputs
        .iter()
        .filter(|input| !input.peaks.is_empty())
        .count();
    let peak = inputs
        .iter()
        .filter(|input| !input.muted && !input.peaks.is_empty())
        .map(AudioInput::peak)
        .reduce(f32::max);
    let empty = crate::metrics::History::new(1);
    for (index, title, value, note, color) in [
        (
            0,
            "Inputs",
            inputs.len().to_string(),
            format!("{metered} have meter data"),
            app.theme.fg,
        ),
        (
            1,
            "Muted",
            muted.to_string(),
            format!("{} unmuted", inputs.len() - muted),
            app.theme.warning,
        ),
        (
            2,
            "Loudest peak",
            peak_text(peak),
            "unmuted / latest event".to_string(),
            app.theme.accent,
        ),
    ] {
        views::metric_tile(
            frame,
            regions[index],
            title,
            value,
            note,
            color,
            &empty,
            app,
        );
    }
}

fn peak_text(peak: Option<f32>) -> String {
    match peak {
        None => "not reported".to_string(),
        Some(peak) if !peak.is_finite() => "not reported".to_string(),
        Some(peak) if peak <= 0.0 => "-inf dBFS".to_string(),
        Some(peak) => format!("{:.1} dBFS", util::linear_to_db(peak)),
    }
}

fn input_detail(input: Option<&AudioInput>, theme: &Theme, unicode: bool) -> Vec<Line<'static>> {
    let Some(input) = input else {
        return vec![chrome::placeholder("no input selected", theme)];
    };
    let mut rows = vec![
        Line::from(Span::styled(
            input.name.clone(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        chrome::row(
            "State",
            if input.muted { "muted" } else { "unmuted" },
            theme.fg,
            12,
            theme,
        ),
        chrome::row(
            "Fader",
            if !input.volume_db.is_finite() || input.volume_db <= -100.0 {
                "-inf dB".to_string()
            } else {
                format!("{:+.1} dB", input.volume_db)
            },
            theme.fg,
            12,
            theme,
        ),
        chrome::row(
            "Channels",
            input.peaks.len().to_string(),
            theme.muted,
            12,
            theme,
        ),
        Line::from(""),
        chrome::section("Source kind", 30, theme, unicode),
        Line::from(Span::styled(
            input.kind.clone(),
            Style::default().fg(theme.muted),
        )),
        Line::from(""),
        chrome::section("Channel peaks", 30, theme, unicode),
    ];
    if input.peaks.is_empty() {
        rows.push(chrome::placeholder("no meter event received", theme));
    } else if input.muted {
        rows.push(chrome::placeholder("muted / meter suppressed", theme));
    } else {
        rows.extend(input.peaks.iter().enumerate().map(|(index, peak)| {
            chrome::row(
                &format!("Channel {}", index + 1),
                peak_text(Some(*peak)),
                theme.fg,
                12,
                theme,
            )
        }));
    }
    rows
}

/// Account for both the list cursor and the meter's fixed columns.
/// Shrink names before sacrificing the numeric peak on compact terminals.
fn row_dimensions(width: u16) -> (usize, usize) {
    let available = width.saturating_sub(2) as usize;
    let name = NAME_WIDTH.min(available.saturating_sub(28)).max(4);
    let bar = available.saturating_sub(20 + name).min(48);
    (name, bar)
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

    fn render_text(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn unknown_meter_data_is_not_claimed_to_be_silence() {
        assert_eq!(peak_text(None), "not reported");
        assert_eq!(peak_text(Some(0.0)), "-inf dBFS");
        assert_eq!(peak_text(Some(0.1)), "-20.0 dBFS");
        let mut quiet = input("Mic", false, 0.0);
        quiet.peaks.clear();
        let text =
            crate::tui::testing::lines_text(&input_detail(Some(&quiet), theme::FALLBACK, false));
        assert!(text.contains("no meter event received"));
    }

    #[test]
    fn audio_rows_leave_room_for_cursor_and_numeric_peak() {
        for width in [30, 40, 60, 80, 82, 120] {
            let (name, bar) = row_dimensions(width);
            let line = meter::row(&input("Mic", false, 0.1), name, bar, theme::FALLBACK, false);
            assert!(
                line.width() + 2 <= width as usize,
                "row overflows at {width}"
            );
            assert!(line_text(&line).contains("-20"));
        }
    }

    #[test]
    fn audio_table_header_is_not_a_clickable_input() {
        let mut app = App::new(crate::config::Config::default());
        app.view = View::Audio;
        app.snapshot.inputs = vec![
            input("First mic", false, 0.1),
            input("Second mic", true, 1.0),
        ];
        let text = render_text(&mut app, 120, 30);
        let region = app.regions.list.unwrap();
        assert!(
            text.lines()
                .nth(region.y as usize)
                .unwrap()
                .contains("First mic")
        );
        assert!(!app.click_row(region.y - 1));
        assert!(app.click_row(region.y + 1));
        assert_eq!(app.selection(View::Audio), 1);
        let text = render_text(&mut app, 120, 30);
        assert!(text.contains("Selected input"));
        assert!(text.contains("meter suppressed"));
    }

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

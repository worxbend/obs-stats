//! The Performance view — frame pacing in detail.
//!
//! Two charts (achieved FPS, average render time) over a counters panel that
//! separates the three ways OBS loses a frame: the renderer skipping, the
//! encoder skipping, and the network dropping. They have different causes and
//! different fixes, so they are never merged into one "dropped frames" number.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::Snapshot;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::views;
use crate::tui::views::loss::Loss;
use crate::tui::views::{axis_count, axis_millis, bar_width, line_chart};
use crate::tui::widgets::chrome;
use crate::util;

const LABEL_WIDTH: usize = 18;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let measured = snapshot.last_poll.is_some() || !snapshot.history.fps.is_empty();
    let [status, area] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let status_text = if !measured {
        "Awaiting first OBS poll / no measurements"
    } else if app.telemetry_stale() {
        "STALE / Retained telemetry, not current measurements"
    } else if app.paused {
        "Paused / retained telemetry"
    } else {
        "Frame pacing / retained-window statistics"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            status_text,
            Style::default().fg(theme.muted),
        ))),
        status,
    );

    // A stable headline -> timeline -> accounting rhythm. The process card
    // is secondary context, never confused with the three frame-loss causes.
    let [charts, details] =
        Layout::vertical([Constraint::Min(10), Constraint::Length(10)]).areas(area);
    let [counters, process] = if area.width >= views::NARROW_WIDTH {
        Layout::horizontal([Constraint::Min(40), Constraint::Length(36)]).areas(details)
    } else {
        [details, Rect::default()]
    };

    let chart_regions = if charts.width >= 64 {
        Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(charts)
    } else {
        Layout::vertical([Constraint::Ratio(1, 2); 2]).split(charts)
    };

    let target = snapshot.video.target_fps();
    let fps_block = chrome::panel_with_note(
        "Frames per second",
        if target > 0.0 {
            format!("target {target:.2}")
        } else {
            "target unknown".to_string()
        },
        false,
        theme,
        app.unicode,
    );
    let fps_inner = chrome::frame_panel(frame, chart_regions[0], fps_block);
    let [headline, fps_plot, summary] = metric_layout(fps_inner);
    frame.render_widget(
        Paragraph::new(Line::from(views::value_unit(
            if measured {
                format!("{:.2}", snapshot.stats.active_fps)
            } else {
                "awaiting".to_string()
            },
            "fps / achieved",
            if measured {
                theme.color_when_low(snapshot.fps_ratio() * 100.0, 95.0, 85.0)
            } else {
                theme.muted
            },
            theme,
        ))),
        headline,
    );
    frame.render_widget(
        Paragraph::new(history_summary(
            &snapshot.history.fps,
            "fps",
            summary.width,
            theme,
        )),
        summary,
    );
    line_chart(
        frame,
        fps_plot,
        &snapshot.history.fps,
        "fps",
        axis_count,
        target.max(30.0),
        theme.color_when_low(snapshot.fps_ratio() * 100.0, 95.0, 85.0),
        theme,
        app.unicode,
    );

    let budget_ms = if target > 0.0 { 1000.0 / target } else { 0.0 };
    let render_block = chrome::panel_with_note(
        "Frame render time",
        if budget_ms > 0.0 {
            format!("budget {budget_ms:.1} ms")
        } else {
            "budget unknown".to_string()
        },
        false,
        theme,
        app.unicode,
    );
    let render_inner = chrome::frame_panel(frame, chart_regions[1], render_block);
    let [headline, render_plot, summary] = metric_layout(render_inner);
    frame.render_widget(
        Paragraph::new(Line::from(views::value_unit(
            if measured {
                format!("{:.2}", snapshot.stats.average_frame_render_time)
            } else {
                "awaiting".to_string()
            },
            "ms / average frame",
            if measured {
                theme.color_when_high(snapshot.frame_budget_ratio() * 100.0, 50.0, 80.0)
            } else {
                theme.muted
            },
            theme,
        ))),
        headline,
    );
    frame.render_widget(
        Paragraph::new(history_summary(
            &snapshot.history.frame_time,
            "ms",
            summary.width,
            theme,
        )),
        summary,
    );
    line_chart(
        frame,
        render_plot,
        &snapshot.history.frame_time,
        "ms",
        axis_millis,
        budget_ms.max(1.0),
        theme.color_when_high(snapshot.frame_budget_ratio() * 100.0, 50.0, 80.0),
        theme,
        app.unicode,
    );

    let counters_block = chrome::panel_with_note(
        "Frame accounting",
        if snapshot.stream.active && snapshot.connection.is_connected() {
            "this stream"
        } else {
            "since OBS started"
        },
        false,
        theme,
        app.unicode,
    );
    let counters_inner = chrome::frame_panel(frame, counters, counters_block);
    frame.render_widget(
        Paragraph::new(if measured {
            counter_rows(snapshot, theme, counters.width, app.unicode)
        } else {
            vec![chrome::placeholder("Awaiting frame counters", theme)]
        }),
        counters_inner,
    );
    if process.width > 0 {
        let inner = chrome::frame_panel(
            frame,
            process,
            chrome::panel_with_note("OBS process", "latest poll", false, theme, app.unicode),
        );
        frame.render_widget(
            Paragraph::new(if measured {
                process_rows(snapshot, theme)
            } else {
                vec![chrome::placeholder("Awaiting process measurements", theme)]
            }),
            inner,
        );
    }
}

/// Leave the chart a real plot area; headline and sample summary never sit
/// over the series. Compact cards prioritise the current measurement.
fn metric_layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(u16::from(area.height >= 5)),
    ])
    .areas(area)
}

fn history_summary(
    history: &crate::metrics::History,
    unit: &str,
    width: u16,
    theme: &Theme,
) -> Line<'static> {
    let text = if history.is_empty() {
        "No retained samples".to_string()
    } else if width < 50 {
        format!(
            "avg {:.2} / peak {:.2} {unit}",
            history.mean(),
            history.max()
        )
    } else {
        format!(
            "avg {:.2} / peak {:.2} {unit} / {} samples",
            history.mean(),
            history.max(),
            history.len()
        )
    };
    Line::from(Span::styled(text, Style::default().fg(theme.muted)))
}

fn process_rows(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    let target = snapshot.video.target_fps();
    let budget = if target > 0.0 {
        format!("{:.1}% used", snapshot.frame_budget_ratio() * 100.0)
    } else {
        "target unknown".to_string()
    };
    vec![
        chrome::row(
            "CPU",
            util::percent(snapshot.stats.cpu_usage),
            theme.accent,
            14,
            theme,
        ),
        chrome::row(
            "Memory",
            util::bytes(snapshot.stats.memory_bytes() as u64),
            theme.fg,
            14,
            theme,
        ),
        chrome::row("Frame budget", budget, theme.fg, 14, theme),
        Line::from(""),
        chrome::row(
            "Canvas",
            if snapshot.video.base_width > 0 && snapshot.video.base_height > 0 {
                format!(
                    "{}x{}",
                    snapshot.video.base_width, snapshot.video.base_height
                )
            } else {
                "unknown".to_string()
            },
            theme.muted,
            14,
            theme,
        ),
        chrome::row(
            "Output",
            if snapshot.video.output_width > 0 && snapshot.video.output_height > 0 {
                format!(
                    "{}x{}",
                    snapshot.video.output_width, snapshot.video.output_height
                )
            } else {
                "unknown".to_string()
            },
            theme.muted,
            14,
            theme,
        ),
        Line::from(Span::styled(
            "OBS process, not host-wide",
            Style::default().fg(theme.muted).add_modifier(Modifier::DIM),
        )),
    ]
}

/// The frame-accounting rows.
///
/// `bar_span` is the panel width the gauges size themselves against.
pub fn counter_rows(
    snapshot: &Snapshot,
    theme: &Theme,
    bar_span: u16,
    unicode: bool,
) -> Vec<Line<'static>> {
    let width = bar_width(bar_span, LABEL_WIDTH, 18);
    let render_skip = Loss::render_skip(snapshot);
    let output_skip = Loss::output_skip(snapshot);
    let network_drop = Loss::network_drop(&snapshot.stream);

    vec![
        chrome::gauge_row(
            "Renderer skipped",
            // Deliberately not just `render_skip.ratio()`: the renderer gauge
            // also reflects how far FPS has fallen below target, so it fills
            // even when nothing has been counted as skipped yet.
            snapshot
                .fps_ratio()
                .mul_add(-1.0, 1.0)
                .max(render_skip.ratio()),
            render_skip.text(),
            render_skip.color(theme),
            LABEL_WIDTH,
            width,
            theme,
            unicode,
        ),
        chrome::gauge_row(
            "Encoder skipped",
            output_skip.ratio(),
            output_skip.text(),
            output_skip.color(theme),
            LABEL_WIDTH,
            width,
            theme,
            unicode,
        ),
        chrome::gauge_row(
            "Network dropped",
            network_drop.ratio(),
            network_drop.text(),
            network_drop.color(theme),
            LABEL_WIDTH,
            width,
            theme,
            unicode,
        ),
        Line::from(""),
        chrome::row(
            "Rendered total",
            snapshot.stats.render_total_frames.to_string(),
            theme.muted,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Encoded total",
            snapshot.stats.output_total_frames.to_string(),
            theme.muted,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Average render",
            util::frame_time(snapshot.stats.average_frame_render_time),
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        Line::from(Span::styled(
            "Frame totals above are since OBS launch",
            Style::default().fg(theme.muted),
        )),
    ]
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
    fn unmeasured_and_retained_performance_are_distinct() {
        let mut app = App::new(crate::config::Config::default());
        let waiting = render_text(&mut app, 120, 30);
        assert!(waiting.contains("Awaiting first OBS poll"));
        assert!(!waiting.contains("0.00"));
        assert!(!waiting.contains("0 (0.0%)"));
        app.snapshot = snapshot();
        app.snapshot.history.fps.push(59.0);
        let retained = render_text(&mut app, 120, 30);
        assert!(retained.contains("Retained telemetry"));
        assert!(retained.contains("59.90 fps"));
        assert!(retained.contains("OBS process"));
    }

    #[test]
    fn compact_performance_keeps_both_headlines_and_accounting() {
        let mut app = App::new(crate::config::Config::default());
        app.snapshot = snapshot();
        app.snapshot.history.fps.push(59.0);
        app.snapshot.history.frame_time.push(3.2);
        let text = render_text(&mut app, 80, 24);
        assert!(text.contains("59.90 fps"), "{text}");
        assert!(text.contains("3.20 ms"), "{text}");
        assert!(text.contains("Renderer skipped"));
        assert!(text.contains("Network dropped"));
        assert!(
            text.contains("peak"),
            "retained stats survive compact layout: {text}"
        );
    }

    #[test]
    fn frame_history_summary_reports_the_observed_window_only() {
        let mut history = crate::metrics::History::new(8);
        history.push(50.0);
        history.push(60.0);
        let text =
            crate::tui::testing::line_text(&history_summary(&history, "fps", 70, theme::FALLBACK));
        assert!(text.contains("avg 55.00"));
        assert!(text.contains("peak 60.00"));
        assert!(text.contains("2 samples"));
    }

    use crate::obs::stats::{Stats, StreamStatus, VideoSettings};
    use crate::tui::testing::lines_text;
    use crate::tui::theme;

    fn snapshot() -> Snapshot {
        let mut snapshot = Snapshot::new(64);
        snapshot.video = VideoSettings {
            fps_numerator: 60,
            fps_denominator: 1,
            ..VideoSettings::default()
        };
        snapshot.stats = Stats {
            active_fps: 59.9,
            average_frame_render_time: 3.2,
            render_skipped_frames: 10,
            render_total_frames: 10_000,
            output_skipped_frames: 5,
            output_total_frames: 9_000,
            ..Stats::default()
        };
        snapshot
    }

    #[test]
    fn the_three_loss_channels_are_reported_separately() {
        let rendered = lines_text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
        assert!(rendered.contains("Renderer skipped"));
        assert!(rendered.contains("Encoder skipped"));
        assert!(rendered.contains("Network dropped"));
    }

    #[test]
    fn counters_show_both_an_absolute_count_and_a_rate() {
        let rendered = lines_text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
        assert!(rendered.contains("10 (0.1%)"), "{rendered}");
        assert!(rendered.contains("5 (0.1%)"), "{rendered}");
    }

    #[test]
    fn totals_and_average_render_time_are_shown() {
        let rendered = lines_text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
        assert!(rendered.contains("10000"));
        assert!(rendered.contains("9000"));
        assert!(rendered.contains("3.20 ms"));
    }

    #[test]
    fn a_clean_run_is_green_and_a_lossy_one_is_red() {
        let palette = theme::FALLBACK;
        let clean = counter_rows(&snapshot(), palette, 100, true);
        assert_eq!(
            clean[1].spans.last().unwrap().style.fg,
            Some(palette.success)
        );

        let mut lossy = snapshot();
        lossy.stats.output_skipped_frames = 500;
        let rows = counter_rows(&lossy, palette, 100, true);
        assert_eq!(rows[1].spans.last().unwrap().style.fg, Some(palette.danger));
    }

    #[test]
    fn network_drops_come_from_the_stream_output_not_the_encoder() {
        let mut snapshot = snapshot();
        snapshot.stream = StreamStatus {
            active: true,
            skipped_frames: 12,
            total_frames: 1200,
            ..StreamStatus::default()
        };
        let rendered = lines_text(&counter_rows(&snapshot, theme::FALLBACK, 100, true));
        assert!(rendered.contains("12 (1.0%)"));
    }

    #[test]
    fn rows_render_in_a_terminal_too_narrow_for_bars() {
        let rows = counter_rows(&snapshot(), theme::FALLBACK, 30, true);
        let rendered = lines_text(&rows);
        assert!(rendered.contains("Renderer skipped"));
        assert!(!rendered.contains('█'), "no room for a bar at this width");
    }

    #[test]
    fn an_empty_snapshot_renders_zeroes_rather_than_panicking() {
        let rendered = lines_text(&counter_rows(&Snapshot::new(8), theme::FALLBACK, 100, true));
        assert!(rendered.contains("0 (0.0%)"));
        assert!(rendered.contains("0.00 ms"));
    }
}

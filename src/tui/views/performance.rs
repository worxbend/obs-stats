//! The Performance view — frame pacing in detail.
//!
//! Two charts (achieved FPS, average render time) over a counters panel that
//! separates the three ways OBS loses a frame: the renderer skipping, the
//! encoder skipping, and the network dropping. They have different causes and
//! different fixes, so they are never merged into one "dropped frames" number.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::metrics::Snapshot;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::views::{NARROW_WIDTH, axis_count, axis_millis, bar_width, line_chart};
use crate::tui::widgets::chrome;
use crate::util;

const LABEL_WIDTH: usize = 18;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;

    let [charts, counters] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(9)]).areas(area);

    let chart_regions = if area.width < NARROW_WIDTH {
        Layout::vertical([Constraint::Ratio(1, 2); 2]).split(charts)
    } else {
        Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(charts)
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
    let fps_inner = fps_block.inner(chart_regions[0]);
    frame.render_widget(fps_block, chart_regions[0]);
    line_chart(
        frame,
        fps_inner,
        &snapshot.history.fps,
        "fps",
        axis_count,
        target.max(30.0),
        theme.scale_down(snapshot.fps_ratio() * 100.0, 95.0, 85.0),
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
    let render_inner = render_block.inner(chart_regions[1]);
    frame.render_widget(render_block, chart_regions[1]);
    line_chart(
        frame,
        render_inner,
        &snapshot.history.frame_time,
        "ms",
        axis_millis,
        budget_ms.max(1.0),
        theme.scale_up(snapshot.frame_budget_ratio() * 100.0, 50.0, 80.0),
        theme,
        app.unicode,
    );

    let counters_block = chrome::panel_with_note(
        "Frame accounting",
        if snapshot.stream.active {
            "this stream"
        } else {
            "since OBS started"
        },
        false,
        theme,
        app.unicode,
    );
    let counters_inner = counters_block.inner(counters);
    frame.render_widget(counters_block, counters);
    frame.render_widget(
        Paragraph::new(counter_rows(snapshot, theme, area.width, app.unicode)),
        counters_inner,
    );
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
    let render_skip = snapshot.session_render_skip_percent();
    let output_skip = snapshot.session_output_skip_percent();
    let network_drop = snapshot.stream.drop_percent();

    vec![
        chrome::gauge_row(
            "Renderer skipped",
            snapshot
                .fps_ratio()
                .mul_add(-1.0, 1.0)
                .max(render_skip / 100.0),
            format!(
                "{} ({})",
                snapshot.session_render_skipped(),
                util::percent(render_skip)
            ),
            theme.scale_up(render_skip, 0.5, 2.0),
            LABEL_WIDTH,
            width,
            theme,
            unicode,
        ),
        chrome::gauge_row(
            "Encoder skipped",
            output_skip / 100.0,
            format!(
                "{} ({})",
                snapshot.session_output_skipped(),
                util::percent(output_skip)
            ),
            theme.scale_up(output_skip, 0.5, 1.0),
            LABEL_WIDTH,
            width,
            theme,
            unicode,
        ),
        chrome::gauge_row(
            "Network dropped",
            network_drop / 100.0,
            format!(
                "{} ({})",
                snapshot.stream.skipped_frames,
                util::percent(network_drop)
            ),
            theme.scale_up(network_drop, 0.1, 1.0),
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::stats::{Stats, StreamStatus, VideoSettings};
    use crate::tui::theme;

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

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
        let rendered = text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
        assert!(rendered.contains("Renderer skipped"));
        assert!(rendered.contains("Encoder skipped"));
        assert!(rendered.contains("Network dropped"));
    }

    #[test]
    fn counters_show_both_an_absolute_count_and_a_rate() {
        let rendered = text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
        assert!(rendered.contains("10 (0.1%)"), "{rendered}");
        assert!(rendered.contains("5 (0.1%)"), "{rendered}");
    }

    #[test]
    fn totals_and_average_render_time_are_shown() {
        let rendered = text(&counter_rows(&snapshot(), theme::FALLBACK, 100, true));
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
        let rendered = text(&counter_rows(&snapshot, theme::FALLBACK, 100, true));
        assert!(rendered.contains("12 (1.0%)"));
    }

    #[test]
    fn rows_render_in_a_terminal_too_narrow_for_bars() {
        let rows = counter_rows(&snapshot(), theme::FALLBACK, 30, true);
        let rendered = text(&rows);
        assert!(rendered.contains("Renderer skipped"));
        assert!(!rendered.contains('█'), "no room for a bar at this width");
    }

    #[test]
    fn an_empty_snapshot_renders_zeroes_rather_than_panicking() {
        let rendered = text(&counter_rows(&Snapshot::new(8), theme::FALLBACK, 100, true));
        assert!(rendered.contains("0 (0.0%)"));
        assert!(rendered.contains("0.00 ms"));
    }
}

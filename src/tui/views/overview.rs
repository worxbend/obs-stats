//! The Overview deck — the btop-style "is everything fine?" screen.
//!
//! Three cards across the top (system, video, health), the two output cards
//! below them, and a bitrate strip at the bottom. On a narrow terminal the
//! cards stack instead of shrinking into illegibility.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::metrics::{Health, Snapshot};
use crate::obs::stats::{DISK_DANGER_MIB, DISK_WARN_MIB};
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::views;
use crate::tui::views::loss::Loss;
use crate::tui::views::{NARROW_WIDTH, axis_bitrate, bar_width, line_chart, sparkline, verdict};
use crate::tui::widgets::chrome;
use crate::util;

const LABEL_WIDTH: usize = 7;

/// Rows a card's trailing sparkline occupies.
///
/// Capped deliberately: `Sparkline` fills every row below the sample, so
/// handing it ten rows produces a wall of solid blocks rather than a graph.
const SPARK_HEIGHT: u16 = 4;

/// CPU percentages at which the gauge turns amber, then red.
const CPU_WARN: f64 = 50.0;
const CPU_DANGER: f64 = 80.0;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let narrow = area.width < NARROW_WIDTH;

    let [top, middle, bottom] = if narrow {
        Layout::vertical([
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(7),
        ])
        .areas(area)
    } else {
        Layout::vertical([
            Constraint::Percentage(42),
            Constraint::Percentage(24),
            Constraint::Percentage(34),
        ])
        .areas(area)
    };

    render_cards(frame, top, app);
    render_outputs(frame, middle, app);
    render_bitrate(frame, bottom, app);
}

/// System / Video / Health, side by side or stacked.
fn render_cards(frame: &mut Frame, area: Rect, app: &App) {
    let regions = views::deck(
        area,
        &[Constraint::Ratio(1, 3); 3],
        &[Constraint::Ratio(1, 3); 3],
    );

    system_card(frame, regions[0], app);
    video_card(frame, regions[1], app);
    health_card(frame, regions[2], app);
}

fn system_card(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let stats = &snapshot.stats;
    let block = chrome::panel("System", false, theme, app.unicode);
    let inner = chrome::frame_panel(frame, area, block);

    if inner.height == 0 {
        return;
    }

    let width = bar_width(area.width, LABEL_WIDTH, 10);
    let mut lines = vec![
        chrome::gauge_row(
            "CPU",
            stats.cpu_usage / 100.0,
            util::percent(stats.cpu_usage),
            theme.color_when_high(stats.cpu_usage, CPU_WARN, CPU_DANGER),
            LABEL_WIDTH,
            width,
            theme,
            app.unicode,
        ),
        chrome::row(
            "MEM",
            util::bytes(stats.memory_bytes() as u64),
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "DISK",
            util::bytes(stats.available_disk_bytes() as u64),
            theme.color_when_low(stats.available_disk_space, DISK_WARN_MIB, DISK_DANGER_MIB),
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "WS I/O",
            format!(
                "{} in  {} out",
                stats.websocket_incoming_messages, stats.websocket_outgoing_messages
            ),
            theme.muted,
            LABEL_WIDTH,
            theme,
        ),
    ];

    let text_rows = (lines.len() as u16).min(inner.height);
    lines.truncate(text_rows as usize);

    let [text_area, _gap, spark_area] = card_rows(inner, text_rows);
    frame.render_widget(Paragraph::new(lines), text_area);
    sparkline(
        frame,
        spark_area,
        &snapshot.history.cpu,
        100.0,
        theme.accent,
        app.unicode,
    );
}

/// Split a card's interior into its text rows, a flexible gap, and a
/// bottom-anchored sparkline strip.
fn card_rows(inner: Rect, text_rows: u16) -> [Rect; 3] {
    // The sparkline only appears once the text and a blank row have been
    // placed; in a short card it collapses to nothing rather than crowding.
    let spark = SPARK_HEIGHT.min(inner.height.saturating_sub(text_rows + 1));
    Layout::vertical([
        Constraint::Length(text_rows),
        Constraint::Min(0),
        Constraint::Length(spark),
    ])
    .areas(inner)
}

fn video_card(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let block = chrome::panel("Video", false, theme, app.unicode);
    let inner = chrome::frame_panel(frame, area, block);

    if inner.height == 0 {
        return;
    }

    let glyphs = chrome::glyphs(app.unicode);
    let target = snapshot.video.target_fps();
    let fps_ratio = snapshot.fps_ratio();
    let budget = snapshot.frame_budget_ratio();
    let width = bar_width(area.width, LABEL_WIDTH, 10);

    let lines = vec![
        chrome::gauge_row(
            "FPS",
            fps_ratio,
            format!("{:.1}", snapshot.stats.active_fps),
            theme.color_when_low(fps_ratio * 100.0, 95.0, 85.0),
            LABEL_WIDTH,
            width,
            theme,
            app.unicode,
        ),
        chrome::gauge_row(
            "FRAME",
            budget,
            util::frame_time(snapshot.stats.average_frame_render_time),
            theme.color_when_high(budget * 100.0, 50.0, 80.0),
            LABEL_WIDTH,
            width,
            theme,
            app.unicode,
        ),
        chrome::row(
            "TARGET",
            if target > 0.0 {
                format!("{target:.2} fps")
            } else {
                "unknown".to_string()
            },
            theme.muted,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "CANVAS",
            if snapshot.video.base_width == 0 {
                "—".to_string()
            } else {
                snapshot.video.geometry(glyphs.arrow)
            },
            if snapshot.video.is_downscaled() {
                theme.info
            } else {
                theme.muted
            },
            LABEL_WIDTH,
            theme,
        ),
    ];

    let text_rows = (lines.len() as u16).min(inner.height);
    let [text_area, _gap, spark_area] = card_rows(inner, text_rows);

    frame.render_widget(Paragraph::new(lines), text_area);
    sparkline(
        frame,
        spark_area,
        &snapshot.history.fps,
        target.max(30.0),
        theme.accent_alt,
        app.unicode,
    );
}

fn health_card(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let health = snapshot.health();
    let color = health_color(health, theme);

    let block = chrome::panel("Health", false, theme, app.unicode);
    let inner = chrome::frame_panel(frame, area, block);

    if inner.height == 0 {
        return;
    }

    let mut lines = vec![verdict(health.label(), color), Line::from("")];
    lines.extend(health_rows(snapshot, theme));

    let text_rows = (lines.len() as u16).min(inner.height);
    let [text_area, _gap, spark_area] = card_rows(inner, text_rows);
    frame.render_widget(Paragraph::new(lines), text_area);
    // Congestion is the leading indicator behind the verdict, so its trend
    // belongs on this card rather than one of the others.
    sparkline(
        frame,
        spark_area,
        &snapshot.history.congestion,
        100.0,
        color,
        app.unicode,
    );
}

/// The detail rows under the verdict.
///
/// Extracted so the wording and colouring can be asserted on directly.
pub fn health_rows(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    let congestion = snapshot.stream.congestion_percent();
    let dropped = Loss::network_drop(&snapshot.stream);
    let render_skip = Loss::render_skip(snapshot);
    let output_skip = Loss::output_skip(snapshot);
    vec![
        chrome::row(
            "DROPPED",
            dropped.text(),
            dropped.color(theme),
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "SKIPPED",
            render_skip.text(),
            render_skip.color(theme),
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "ENCODER",
            output_skip.text(),
            output_skip.color(theme),
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "CONGEST",
            util::percent(congestion),
            theme.color_when_high(congestion, 20.0, 50.0),
            LABEL_WIDTH + 2,
            theme,
        ),
    ]
}

/// Stream and recording cards.
fn render_outputs(frame: &mut Frame, area: Rect, app: &App) {
    let regions = views::deck(
        area,
        &[Constraint::Ratio(1, 2); 2],
        &[Constraint::Ratio(1, 2); 2],
    );

    let theme = app.theme;
    let snapshot = &app.snapshot;

    let stream_block = chrome::panel_with_note(
        "Stream",
        if snapshot.stream.active {
            "LIVE"
        } else {
            "idle"
        },
        false,
        theme,
        app.unicode,
    );
    let stream_inner = chrome::frame_panel(frame, regions[0], stream_block);
    frame.render_widget(Paragraph::new(stream_rows(snapshot, theme)), stream_inner);

    let record_block = chrome::panel_with_note(
        "Recording",
        match (snapshot.record.active, snapshot.record.paused) {
            (true, true) => "paused",
            (true, false) => "REC",
            (false, _) => "idle",
        },
        false,
        theme,
        app.unicode,
    );
    let record_inner = chrome::frame_panel(frame, regions[1], record_block);
    frame.render_widget(Paragraph::new(record_rows(snapshot, theme)), record_inner);
}

/// Rows of the stream card.
pub fn stream_rows(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    if !snapshot.stream.active {
        return vec![Line::from(""), chrome::placeholder("not streaming", theme)];
    }

    vec![
        chrome::row(
            "ELAPSED",
            util::duration_ms(snapshot.stream.duration_ms),
            theme.fg,
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "BITRATE",
            util::bitrate(snapshot.stream_bitrate),
            theme.accent,
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "SENT",
            util::bytes(snapshot.stream.bytes),
            theme.muted,
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "FRAMES",
            format!(
                "{} sent, {} dropped",
                snapshot.stream.total_frames, snapshot.stream.skipped_frames
            ),
            Loss::network_drop(&snapshot.stream).color(theme),
            LABEL_WIDTH + 2,
            theme,
        ),
    ]
}

/// Rows of the recording card.
pub fn record_rows(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    if !snapshot.record.active {
        return vec![Line::from(""), chrome::placeholder("not recording", theme)];
    }

    vec![
        chrome::row(
            "ELAPSED",
            util::duration_ms(snapshot.record.duration_ms),
            theme.fg,
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "WRITING",
            if snapshot.record.paused {
                "paused".to_string()
            } else {
                util::bitrate(snapshot.record_bitrate)
            },
            if snapshot.record.paused {
                theme.warning
            } else {
                theme.info
            },
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "ON DISK",
            util::bytes(snapshot.record.bytes),
            theme.muted,
            LABEL_WIDTH + 2,
            theme,
        ),
        chrome::row(
            "TIMECODE",
            if snapshot.record.timecode.is_empty() {
                "—".to_string()
            } else {
                snapshot.record.timecode.clone()
            },
            theme.muted,
            LABEL_WIDTH + 2,
            theme,
        ),
    ]
}

fn render_bitrate(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let history = &app.snapshot.history.stream_bitrate;
    let block = chrome::panel_with_note(
        "Stream bitrate",
        util::bitrate(app.snapshot.stream_bitrate),
        false,
        theme,
        app.unicode,
    );
    let inner = chrome::frame_panel(frame, area, block);

    // 1 Mbps floor keeps an idle chart flat instead of amplifying noise.
    line_chart(
        frame,
        inner,
        history,
        "bitrate",
        axis_bitrate,
        1_000_000.0,
        theme.accent,
        theme,
        app.unicode,
    );
}

fn health_color(health: Health, theme: &Theme) -> ratatui::style::Color {
    match health {
        Health::Healthy => theme.success,
        Health::Strained => theme.warning,
        Health::Dropping => theme.danger,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Snapshot;
    use crate::obs::stats::{RecordStatus, StreamStatus, VideoSettings};
    use crate::tui::testing::lines_text;
    use crate::tui::theme;

    fn streaming_snapshot() -> Snapshot {
        let mut snapshot = Snapshot::new(64);
        snapshot.video = VideoSettings {
            fps_numerator: 60,
            fps_denominator: 1,
            base_width: 1920,
            base_height: 1080,
            output_width: 1920,
            output_height: 1080,
        };
        snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 65_000,
            bytes: 750_000_000,
            total_frames: 3900,
            skipped_frames: 4,
            ..StreamStatus::default()
        };
        snapshot.stream_bitrate = 6_000_000.0;
        snapshot
    }

    #[test]
    fn a_card_anchors_its_sparkline_to_the_bottom_and_caps_its_height() {
        let inner = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 14,
        };
        let [text, _gap, spark] = card_rows(inner, 4);
        assert_eq!(text.height, 4);
        assert_eq!(spark.height, SPARK_HEIGHT);
        assert_eq!(spark.bottom(), inner.bottom(), "must sit on the floor");
    }

    #[test]
    fn a_short_card_drops_the_sparkline_rather_than_crowding_the_text() {
        let inner = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 4,
        };
        let [text, _gap, spark] = card_rows(inner, 4);
        assert_eq!(text.height, 4);
        assert_eq!(spark.height, 0);

        // And a card with no room at all must not underflow.
        let [_, _, none] = card_rows(
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 2,
            },
            4,
        );
        assert_eq!(none.height, 0);
    }

    #[test]
    fn an_idle_stream_card_says_so_instead_of_showing_zeroes() {
        let rendered = lines_text(&stream_rows(&Snapshot::new(8), theme::FALLBACK));
        assert!(rendered.contains("not streaming"));
        assert!(!rendered.contains("BITRATE"));
    }

    #[test]
    fn a_live_stream_card_reports_time_bitrate_and_frames() {
        let rendered = lines_text(&stream_rows(&streaming_snapshot(), theme::FALLBACK));
        assert!(rendered.contains("01:05"));
        assert!(rendered.contains("6.00 Mbps"));
        assert!(rendered.contains("715.3 MiB"));
        assert!(rendered.contains("3900 sent, 4 dropped"));
    }

    #[test]
    fn an_idle_recording_card_says_so() {
        let rendered = lines_text(&record_rows(&Snapshot::new(8), theme::FALLBACK));
        assert!(rendered.contains("not recording"));
    }

    #[test]
    fn a_paused_recording_shows_paused_rather_than_a_write_rate() {
        let mut snapshot = Snapshot::new(8);
        snapshot.record = RecordStatus {
            active: true,
            paused: true,
            duration_ms: 5000,
            bytes: 1024,
            ..RecordStatus::default()
        };
        let rendered = lines_text(&record_rows(&snapshot, theme::FALLBACK));
        assert!(rendered.contains("paused"));
        assert!(!rendered.contains("kbps"));
    }

    #[test]
    fn an_active_recording_reports_its_write_rate_and_size() {
        let mut snapshot = Snapshot::new(8);
        snapshot.record = RecordStatus {
            active: true,
            duration_ms: 5000,
            bytes: 10 * 1024 * 1024,
            timecode: "00:00:05.000".into(),
            ..RecordStatus::default()
        };
        snapshot.record_bitrate = 8_000_000.0;
        let rendered = lines_text(&record_rows(&snapshot, theme::FALLBACK));
        assert!(rendered.contains("8.00 Mbps"));
        assert!(rendered.contains("10.0 MiB"));
        assert!(rendered.contains("00:00:05.000"));
    }

    #[test]
    fn health_rows_cover_every_drop_channel() {
        let rendered = lines_text(&health_rows(&streaming_snapshot(), theme::FALLBACK));
        for label in ["DROPPED", "SKIPPED", "ENCODER", "CONGEST"] {
            assert!(rendered.contains(label), "missing {label}");
        }
    }

    #[test]
    fn health_rows_colour_a_clean_stream_green_and_a_dropping_one_red() {
        let palette = theme::FALLBACK;
        let mut spotless = streaming_snapshot();
        spotless.stream.skipped_frames = 0;
        let clean = health_rows(&spotless, palette);
        assert_eq!(clean[0].spans[1].style.fg, Some(palette.success));

        // Even a 0.1% drop rate is worth an amber row — viewers see it.
        let nearly_clean = health_rows(&streaming_snapshot(), palette);
        assert_eq!(nearly_clean[0].spans[1].style.fg, Some(palette.warning));

        let mut bad = streaming_snapshot();
        bad.stream.skipped_frames = 200;
        let dropping = health_rows(&bad, palette);
        assert_eq!(dropping[0].spans[1].style.fg, Some(palette.danger));
    }

    #[test]
    fn health_colours_track_the_verdict() {
        let palette = theme::FALLBACK;
        assert_eq!(health_color(Health::Healthy, palette), palette.success);
        assert_eq!(health_color(Health::Strained, palette), palette.warning);
        assert_eq!(health_color(Health::Dropping, palette), palette.danger);
    }
}

//! The Stream view — the two outputs in full, with a bitrate chart.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::metrics::Snapshot;
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::views::{NARROW_WIDTH, axis_bitrate, axis_percent, line_chart};
use crate::tui::widgets::chrome;
use crate::util;

const LABEL_WIDTH: usize = 14;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;

    let [charts, details] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(10)]).areas(area);

    let chart_regions = if area.width < NARROW_WIDTH {
        Layout::vertical([Constraint::Ratio(1, 2); 2]).split(charts)
    } else {
        Layout::horizontal([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).split(charts)
    };

    let bitrate_block = chrome::panel_with_note(
        "Bitrate",
        format!(
            "now {} · peak {}",
            util::bitrate(snapshot.stream_bitrate),
            util::bitrate(snapshot.history.stream_bitrate.max())
        ),
        false,
        theme,
        app.unicode,
    );
    let bitrate_inner = bitrate_block.inner(chart_regions[0]);
    frame.render_widget(bitrate_block, chart_regions[0]);
    line_chart(
        frame,
        bitrate_inner,
        &snapshot.history.stream_bitrate,
        "bps",
        axis_bitrate,
        1_000_000.0,
        theme.accent,
        theme,
        app.unicode,
    );

    let congestion = snapshot.stream.congestion * 100.0;
    let congestion_block = chrome::panel_with_note(
        "Congestion",
        util::percent(congestion),
        false,
        theme,
        app.unicode,
    );
    let congestion_inner = congestion_block.inner(chart_regions[1]);
    frame.render_widget(congestion_block, chart_regions[1]);
    line_chart(
        frame,
        congestion_inner,
        &snapshot.history.congestion,
        "%",
        axis_percent,
        100.0,
        theme.color_when_high(congestion, 20.0, 50.0),
        theme,
        app.unicode,
    );

    let detail_regions = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(details);

    let stream_block = chrome::panel("Stream output", false, theme, app.unicode);
    let stream_inner = stream_block.inner(detail_regions[0]);
    frame.render_widget(stream_block, detail_regions[0]);
    frame.render_widget(Paragraph::new(stream_detail(snapshot, theme)), stream_inner);

    let record_block = chrome::panel("Recording output", false, theme, app.unicode);
    let record_inner = record_block.inner(detail_regions[1]);
    frame.render_widget(record_block, detail_regions[1]);
    frame.render_widget(Paragraph::new(record_detail(snapshot, theme)), record_inner);
}

/// Every field of `GetStreamStatus`, plus what we derive from it.
pub fn stream_detail(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    let stream = &snapshot.stream;
    let state = if stream.reconnecting {
        ("reconnecting", theme.warning)
    } else if stream.active {
        ("live", theme.success)
    } else {
        ("idle", theme.muted)
    };

    let mut rows = vec![chrome::row("State", state.0, state.1, LABEL_WIDTH, theme)];
    if !stream.active {
        rows.push(chrome::placeholder(
            "start streaming to see live figures",
            theme,
        ));
        return rows;
    }

    rows.extend([
        chrome::row(
            "Timecode",
            if stream.timecode.is_empty() {
                util::duration_ms(stream.duration_ms)
            } else {
                stream.timecode.clone()
            },
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Bitrate",
            util::bitrate(snapshot.stream_bitrate),
            theme.accent,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Sent",
            util::bytes(stream.bytes),
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Frames",
            stream.total_frames.to_string(),
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Dropped",
            format!(
                "{} ({})",
                stream.skipped_frames,
                util::percent(stream.drop_percent())
            ),
            theme.color_when_high(stream.drop_percent(), 0.1, 1.0),
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Congestion",
            util::percent(stream.congestion * 100.0),
            theme.color_when_high(stream.congestion * 100.0, 20.0, 50.0),
            LABEL_WIDTH,
            theme,
        ),
    ]);
    rows
}

/// Every field of `GetRecordStatus`, plus the derived write rate.
pub fn record_detail(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    let record = &snapshot.record;
    let state = match (record.active, record.paused) {
        (true, true) => ("paused", theme.warning),
        (true, false) => ("recording", theme.info),
        (false, _) => ("idle", theme.muted),
    };

    let mut rows = vec![chrome::row("State", state.0, state.1, LABEL_WIDTH, theme)];
    if !record.active {
        rows.push(chrome::placeholder(
            "start recording to see live figures",
            theme,
        ));
        return rows;
    }

    rows.extend([
        chrome::row(
            "Timecode",
            if record.timecode.is_empty() {
                util::duration_ms(record.duration_ms)
            } else {
                record.timecode.clone()
            },
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Write rate",
            if record.paused {
                "paused".to_string()
            } else {
                util::bitrate(snapshot.record_bitrate)
            },
            if record.paused {
                theme.warning
            } else {
                theme.info
            },
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "On disk",
            util::bytes(record.bytes),
            theme.fg,
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Disk free",
            util::bytes((snapshot.stats.available_disk_space * 1024.0 * 1024.0) as u64),
            theme.color_when_low(snapshot.stats.available_disk_space, 20_000.0, 5_000.0),
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Remaining",
            remaining_estimate(snapshot),
            theme.muted,
            LABEL_WIDTH,
            theme,
        ),
    ]);
    rows
}

/// How much longer the recording can run before the disk fills, at the
/// current write rate.
///
/// Returns `—` when the rate is unknown or effectively zero, since an
/// estimate divided by ~0 would render as a meaningless century.
pub fn remaining_estimate(snapshot: &Snapshot) -> String {
    let bytes_per_second = snapshot.record_bitrate / 8.0;
    if bytes_per_second < 1024.0 {
        return "—".to_string();
    }
    let free_bytes = snapshot.stats.available_disk_space * 1024.0 * 1024.0;
    let seconds = free_bytes / bytes_per_second;
    if !seconds.is_finite() || seconds <= 0.0 {
        return "—".to_string();
    }
    // Cap at 99 hours; past that the number is noise, not information.
    let capped = seconds.min(99.0 * 3600.0);
    format!("~{}", util::duration_ms((capped * 1000.0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::stats::{RecordStatus, Stats, StreamStatus};
    use crate::tui::testing::lines_text;
    use crate::tui::theme;

    #[test]
    fn an_idle_stream_shows_state_and_a_prompt_only() {
        let rendered = lines_text(&stream_detail(&Snapshot::new(8), theme::FALLBACK));
        assert!(rendered.contains("idle"));
        assert!(rendered.contains("start streaming"));
        assert!(!rendered.contains("Bitrate"));
    }

    #[test]
    fn a_live_stream_lists_every_status_field() {
        let mut snapshot = Snapshot::new(8);
        snapshot.stream = StreamStatus {
            active: true,
            timecode: "00:05:00.000".into(),
            duration_ms: 300_000,
            bytes: 1024 * 1024 * 200,
            total_frames: 18_000,
            skipped_frames: 9,
            congestion: 0.05,
            ..StreamStatus::default()
        };
        snapshot.stream_bitrate = 5_500_000.0;
        let rendered = lines_text(&stream_detail(&snapshot, theme::FALLBACK));
        assert!(rendered.contains("live"));
        assert!(rendered.contains("00:05:00.000"));
        assert!(rendered.contains("5.50 Mbps"));
        assert!(rendered.contains("200.0 MiB"));
        assert!(rendered.contains("18000"));
        assert!(rendered.contains("9 (0.1%)"));
        assert!(rendered.contains("5.0%"));
    }

    #[test]
    fn reconnecting_is_distinguished_from_live() {
        let mut snapshot = Snapshot::new(8);
        snapshot.stream = StreamStatus {
            active: true,
            reconnecting: true,
            ..StreamStatus::default()
        };
        let rows = stream_detail(&snapshot, theme::FALLBACK);
        assert_eq!(rows[0].spans[1].content, "reconnecting");
        assert_eq!(rows[0].spans[1].style.fg, Some(theme::FALLBACK.warning));
    }

    #[test]
    fn a_missing_timecode_falls_back_to_the_duration() {
        let mut snapshot = Snapshot::new(8);
        snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 65_000,
            ..StreamStatus::default()
        };
        assert!(lines_text(&stream_detail(&snapshot, theme::FALLBACK)).contains("01:05"));
    }

    #[test]
    fn an_idle_recording_shows_a_prompt() {
        let rendered = lines_text(&record_detail(&Snapshot::new(8), theme::FALLBACK));
        assert!(rendered.contains("idle"));
        assert!(rendered.contains("start recording"));
    }

    #[test]
    fn a_paused_recording_is_labelled_and_shows_no_rate() {
        let mut snapshot = Snapshot::new(8);
        snapshot.record = RecordStatus {
            active: true,
            paused: true,
            ..RecordStatus::default()
        };
        let rendered = lines_text(&record_detail(&snapshot, theme::FALLBACK));
        assert!(rendered.contains("paused"));
        assert!(!rendered.contains("Mbps"));
    }

    #[test]
    fn the_remaining_estimate_divides_free_space_by_the_write_rate() {
        let mut snapshot = Snapshot::new(8);
        // 8 Mbps = 1 MB/s; 3600 MiB free ≈ 1 hour.
        snapshot.record_bitrate = 8.0 * 1024.0 * 1024.0;
        snapshot.stats = Stats {
            available_disk_space: 3600.0,
            ..Stats::default()
        };
        assert_eq!(remaining_estimate(&snapshot), "~1:00:00");
    }

    #[test]
    fn the_remaining_estimate_is_blank_without_a_meaningful_rate() {
        let mut snapshot = Snapshot::new(8);
        snapshot.stats = Stats {
            available_disk_space: 100_000.0,
            ..Stats::default()
        };
        assert_eq!(remaining_estimate(&snapshot), "—", "no write rate yet");

        snapshot.record_bitrate = 1.0;
        assert_eq!(
            remaining_estimate(&snapshot),
            "—",
            "a trickle is not an estimate"
        );
    }

    #[test]
    fn the_remaining_estimate_is_capped_rather_than_absurd() {
        let mut snapshot = Snapshot::new(8);
        snapshot.record_bitrate = 8.0 * 1024.0 * 1024.0;
        snapshot.stats = Stats {
            available_disk_space: 100_000_000.0,
            ..Stats::default()
        };
        assert_eq!(remaining_estimate(&snapshot), "~99:00:00");
    }
}

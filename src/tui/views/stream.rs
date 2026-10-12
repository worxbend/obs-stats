//! The Stream view — the two outputs in full, with a bitrate chart.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::Snapshot;
use crate::obs::stats::{DISK_DANGER_MIB, DISK_WARN_MIB};
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::views;
use crate::tui::views::loss::Loss;
use crate::tui::views::{axis_bitrate, axis_percent, line_chart};
use crate::tui::widgets::chrome;
use crate::util;

const LABEL_WIDTH: usize = 14;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let measured = snapshot.last_poll.is_some() || !snapshot.history.stream_bitrate.is_empty();
    let [status, area] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let status_text = if !measured {
        "Awaiting first OBS poll / output status unknown"
    } else if app.telemetry_stale() {
        "STALE / Retained telemetry, not current measurements"
    } else if app.paused {
        "Paused / retained telemetry"
    } else {
        "Output telemetry / rates derived from byte-counter deltas"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            status_text,
            Style::default().fg(theme.muted),
        ))),
        status,
    );

    let [charts, details] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(if area.height < 25 { 11 } else { 12 }),
    ])
    .areas(area);

    let chart_regions = if charts.width >= 96 {
        Layout::horizontal([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).split(charts)
    } else if charts.width >= 64 {
        Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(charts)
    } else {
        Layout::vertical([Constraint::Ratio(1, 2); 2]).split(charts)
    };

    let bitrate_block = chrome::panel_with_note(
        "Bitrate",
        "measured / not a target",
        false,
        theme,
        app.unicode,
    );
    let bitrate_inner = chrome::frame_panel(frame, chart_regions[0], bitrate_block);
    let [headline, bitrate_plot, summary] = timeline_layout(bitrate_inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if !measured {
                    "awaiting".into()
                } else if app.telemetry_stale() || app.paused {
                    "unavailable".into()
                } else if snapshot.stream.active {
                    util::bitrate(snapshot.stream_bitrate)
                } else {
                    "idle".into()
                },
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / stream throughput", Style::default().fg(theme.muted)),
        ])),
        headline,
    );
    frame.render_widget(
        Paragraph::new(rate_summary(&snapshot.history.stream_bitrate, theme)),
        summary,
    );
    line_chart(
        frame,
        bitrate_plot,
        &snapshot.history.stream_bitrate,
        "bps",
        axis_bitrate,
        1_000_000.0,
        theme.accent,
        theme,
        app.unicode,
    );

    let congestion = snapshot.stream.congestion_percent();
    let congestion_block =
        chrome::panel_with_note("Congestion", "OBS backpressure", false, theme, app.unicode);
    let congestion_inner = chrome::frame_panel(frame, chart_regions[1], congestion_block);
    let [headline, congestion_plot, summary] = timeline_layout(congestion_inner);
    frame.render_widget(
        Paragraph::new(Line::from(views::value_unit(
            if !measured {
                "awaiting".into()
            } else if snapshot.stream.active {
                util::percent(congestion)
            } else {
                "idle".into()
            },
            "/ congestion",
            if measured {
                theme.color_when_high(congestion, 20.0, 50.0)
            } else {
                theme.muted
            },
            theme,
        ))),
        headline,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if snapshot.history.congestion.is_empty() {
                "No retained samples".to_string()
            } else {
                format!(
                    "avg {:.1}% / peak {:.1}%",
                    snapshot.history.congestion.mean(),
                    snapshot.history.congestion.max()
                )
            },
            Style::default().fg(theme.muted),
        ))),
        summary,
    );
    line_chart(
        frame,
        congestion_plot,
        &snapshot.history.congestion,
        "%",
        axis_percent,
        100.0,
        theme.color_when_high(congestion, 20.0, 50.0),
        theme,
        app.unicode,
    );

    let detail_regions = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(details);

    let stream_block = if !measured {
        chrome::panel_with_note("Stream output", "unknown", false, theme, app.unicode)
    } else if app.telemetry_stale() || app.paused {
        chrome::panel_with_badge(
            "Stream output",
            if app.paused { "PAUSED" } else { "STALE" },
            theme.warning,
            false,
            theme,
            app.unicode,
        )
    } else if snapshot.stream.active {
        let (note, color) = if snapshot.stream.reconnecting {
            ("reconnecting", theme.warning)
        } else {
            ("LIVE", theme.danger)
        };
        chrome::panel_with_badge("Stream output", note, color, false, theme, app.unicode)
    } else {
        chrome::panel_with_note("Stream output", "idle", false, theme, app.unicode)
    };
    let stream_inner = chrome::frame_panel(frame, detail_regions[0], stream_block);
    let mut stream_rows = if measured {
        stream_detail(snapshot, theme)
    } else {
        vec![chrome::placeholder("Awaiting output status", theme)]
    };
    stream_rows.push(Line::from(""));
    stream_rows.push(chrome::placeholder(
        "Loss counts / this stream output",
        theme,
    ));
    frame.render_widget(Paragraph::new(stream_rows), stream_inner);

    let record_block = if !measured {
        chrome::panel_with_note("Recording output", "unknown", false, theme, app.unicode)
    } else if app.telemetry_stale() || app.paused {
        chrome::panel_with_badge(
            "Recording output",
            if app.paused { "PAUSED" } else { "STALE" },
            theme.warning,
            false,
            theme,
            app.unicode,
        )
    } else {
        match (snapshot.record.active, snapshot.record.paused) {
            (true, true) => chrome::panel_with_badge(
                "Recording output",
                "paused",
                theme.warning,
                false,
                theme,
                app.unicode,
            ),
            (true, false) => chrome::panel_with_badge(
                "Recording output",
                "REC",
                theme.info,
                false,
                theme,
                app.unicode,
            ),
            (false, _) => {
                chrome::panel_with_note("Recording output", "idle", false, theme, app.unicode)
            }
        }
    };
    let record_inner = chrome::frame_panel(frame, detail_regions[1], record_block);
    let [rows, rate] = Layout::vertical([
        Constraint::Min(6),
        Constraint::Length(if record_inner.height >= 9 { 3 } else { 0 }),
    ])
    .areas(record_inner);
    frame.render_widget(
        Paragraph::new(if measured {
            record_detail(snapshot, theme)
        } else {
            vec![chrome::placeholder("Awaiting output status", theme)]
        }),
        rows,
    );
    let [label, plot] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(rate);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if snapshot.history.record_bitrate.is_empty() {
                "Awaiting write-rate samples"
            } else {
                "Write rate / retained polls"
            },
            Style::default().fg(theme.muted),
        ))),
        label,
    );
    views::sparkline(
        frame,
        plot,
        &snapshot.history.record_bitrate,
        1_000_000.0,
        theme.info,
        app.unicode,
    );
}

fn timeline_layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(u16::from(area.height >= 5)),
    ])
    .areas(area)
}

fn rate_summary(history: &crate::metrics::History, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        views::history_summary(history, util::bitrate),
        Style::default().fg(theme.muted),
    ))
}

/// Every field of `GetStreamStatus`, plus what we derive from it.
pub fn stream_detail(snapshot: &Snapshot, theme: &Theme) -> Vec<Line<'static>> {
    let stream = &snapshot.stream;
    let dropped = Loss::network_drop(stream);
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
            dropped.text(),
            dropped.color(theme),
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Congestion",
            util::percent(stream.congestion_percent()),
            theme.color_when_high(stream.congestion_percent(), 20.0, 50.0),
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
            util::bytes(snapshot.stats.available_disk_bytes() as u64),
            theme.color_when_low(
                snapshot.stats.available_disk_space,
                DISK_WARN_MIB,
                DISK_DANGER_MIB,
            ),
            LABEL_WIDTH,
            theme,
        ),
        chrome::row(
            "Remaining",
            if record.paused {
                "paused".to_string()
            } else {
                remaining_estimate(snapshot)
            },
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
    let free_bytes = snapshot.stats.available_disk_bytes();
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
    fn unmeasured_outputs_are_unknown_not_idle_zeroes() {
        let mut app = App::new(crate::config::Config::default());
        let text = render_text(&mut app, 120, 30);
        assert!(text.contains("output status unknown"));
        assert!(text.contains("Awaiting output status"));
        assert!(!text.contains("0 kbps"));
        assert!(!text.contains("idle"));
    }

    #[test]
    fn compact_stream_keeps_both_outputs_and_retained_rates() {
        let mut app = App::new(crate::config::Config::default());
        app.snapshot.stream.active = true;
        app.snapshot.record.active = true;
        app.snapshot.stream_bitrate = 6_000_000.0;
        app.snapshot.history.stream_bitrate.push(5_000_000.0);
        app.snapshot.history.stream_bitrate.push(7_000_000.0);
        app.snapshot.history.congestion.push(4.0);
        let text = render_text(&mut app, 80, 24);
        assert!(text.contains("6.00 Mbps"), "{text}");
        assert!(text.contains("Stream output"));
        assert!(text.contains("Recording output"));
        assert!(
            text.contains("Remaining"),
            "recording details outrank decorative history: {text}"
        );
        assert!(text.contains("Retained telemetry"));
    }

    #[test]
    fn bitrate_summary_uses_mean_peak_and_real_sample_count() {
        let mut history = crate::metrics::History::new(8);
        assert!(
            crate::tui::testing::line_text(&rate_summary(&history, theme::FALLBACK))
                .contains("awaiting")
        );
        history.push(5_000_000.0);
        history.push(7_000_000.0);
        let text = crate::tui::testing::line_text(&rate_summary(&history, theme::FALLBACK));
        assert!(text.contains("avg 6.00 Mbps"));
        assert!(text.contains("peak 7.00 Mbps"));
        assert!(text.contains("2 samples"));
    }

    #[test]
    fn paused_recording_does_not_reuse_a_retained_rate_for_remaining_time() {
        let mut snapshot = Snapshot::new(8);
        snapshot.record.active = true;
        snapshot.record.paused = true;
        snapshot.record_bitrate = 8_000_000.0;
        snapshot.stats.available_disk_space = 3600.0;
        let rows = record_detail(&snapshot, theme::FALLBACK);
        assert_eq!(rows.last().unwrap().spans[1].content, "paused");
    }

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

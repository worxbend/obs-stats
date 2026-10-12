//! The top bar: identity, connection state, live output badges.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::{ConnectionState, Snapshot};
use crate::tui::app::{App, Hit};
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;
use crate::{APP_NAME, util};

/// Height the header occupies, including its rule.
pub const HEIGHT: u16 = 2;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let [top, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let identity = identity(app);
    let (status, alert_index) = if top.width < 96 {
        compact_status(app)
    } else {
        status(app)
    };
    let status_width = (status.width() as u16).min(top.width.saturating_sub(20));
    let [left, right] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(status_width)]).areas(top);
    let alert_rect = alert_index.and_then(|index| span_rect(&status, index, right));

    frame.render_widget(Paragraph::new(identity), left);
    frame.render_widget(Paragraph::new(status).alignment(Alignment::Right), right);

    if let Some(rect) = alert_rect {
        app.regions.hits.push((rect, Hit::Alerts));
    }

    // A compact workspace breadcrumb replaces the ornamental rainbow rule.
    // The mode is a real application invariant, not a synthetic health claim.
    let [workspace, mode] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(12)]).areas(rule);
    let rail = Style::default().bg(app.theme.surface()).fg(app.theme.muted);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.accent)),
            Span::styled("TELEMETRY / ", Style::default().fg(app.theme.muted)),
            Span::styled(app.view.title(), Style::default().fg(app.theme.fg)),
        ]))
        .style(rail),
        workspace,
    );
    frame.render_widget(
        Paragraph::new("READ ONLY ")
            .alignment(Alignment::Right)
            .style(rail),
        mode,
    );
}

/// `obs-stats v0.1.0 · OBS 30.2.3 (linux) · ws 5.5.0`
fn identity(app: &App) -> Line<'static> {
    let theme = app.theme;
    // A single, steady orange wordmark leaves colour variation to the data.
    let mut spans = vec![Span::styled(
        APP_NAME,
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )];
    spans.push(Span::styled(
        format!(" v{}", app.version),
        Style::default().fg(theme.muted),
    ));

    let version = &app.snapshot.version;
    if !version.obs_version.is_empty() {
        spans.push(dot(theme, app.unicode));
        spans.push(Span::styled(
            format!("OBS {}", version.obs_version),
            Style::default().fg(theme.accent_alt),
        ));
        if !version.platform.is_empty() {
            spans.push(Span::styled(
                format!(" ({})", version.platform),
                Style::default().fg(theme.muted),
            ));
        }
    }
    if !version.websocket_version.is_empty() {
        spans.push(dot(theme, app.unicode));
        spans.push(Span::styled(
            format!("ws {}", version.websocket_version),
            Style::default().fg(theme.muted),
        ));
    }

    Line::from(spans)
}

/// Where the span at `index` lands in a right-aligned `area`.
///
/// `None` when the line is wider than its column: the alignment clips it from
/// the left, so no reported position would be true.
fn span_rect(line: &Line<'static>, index: usize, area: Rect) -> Option<Rect> {
    let widths: Vec<u16> = line.spans.iter().map(|span| span.width() as u16).collect();
    let total: u16 = widths.iter().sum();
    if total > area.width || index >= widths.len() {
        return None;
    }
    let x = area.x + area.width - total + widths[..index].iter().sum::<u16>();
    Some(Rect {
        x,
        y: area.y,
        width: widths[index],
        height: 1,
    })
}

/// Right-hand side: live badges, then the connection chip.
///
/// Also returns which span holds the alert counter, when one is shown. The
/// counter is the only clickable piece of the status line, and it is recorded
/// as it is built — the alternative, searching the finished line for its
/// marker glyph, means the click target depends on the rendered text and
/// breaks the moment two spans start with the same character.
/// On a narrow terminal, state outranks elapsed times and the poll counter.
/// Durations remain in the output cards; the link must never be clipped away.
fn compact_status(app: &App) -> (Line<'static>, Option<usize>) {
    let s = &app.snapshot;
    let t = app.theme;
    let mut spans = chrome::badge(
        &s.connection.label(),
        match s.connection {
            ConnectionState::Connected if app.has_measurements() && !app.telemetry_stale() => {
                t.success
            }
            ConnectionState::Connected | ConnectionState::Connecting { .. } => t.warning,
            _ => t.danger,
        },
        t,
    );
    let usable = app.has_measurements()
        && s.connection.is_connected()
        && !app.telemetry_stale()
        && !app.paused;
    if app.telemetry_stale() || app.paused {
        spans.push(Span::raw(" "));
        spans.extend(chrome::badge(
            if app.telemetry_stale() {
                "STALE"
            } else {
                "PAUSED"
            },
            t.warning,
            t,
        ));
    } else if usable {
        if s.stream.active {
            spans.push(Span::raw(" "));
            spans.extend(chrome::badge(
                if s.stream.reconnecting {
                    "RETRY"
                } else {
                    "LIVE"
                },
                if s.stream.reconnecting {
                    t.warning
                } else {
                    t.danger
                },
                t,
            ));
        }
        if s.record.active {
            spans.push(Span::raw(" "));
            spans.extend(chrome::badge(
                if s.record.paused { "REC PAUSED" } else { "REC" },
                t.info,
                t,
            ));
        }
    } else if s.connection.is_connected() && !app.has_measurements() {
        spans.push(Span::styled(" waiting", Style::default().fg(t.muted)));
    }
    let alert_index = if app.alert_count > 0 {
        let index = spans.len();
        spans.push(Span::styled(
            format!(" !{}", app.alert_count),
            Style::default().fg(t.warning),
        ));
        Some(index)
    } else {
        None
    };
    (Line::from(spans), alert_index)
}

fn status(app: &App) -> (Line<'static>, Option<usize>) {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let mut spans = Vec::new();
    let sampled = app.has_measurements();
    let usable =
        snapshot.connection.is_connected() && sampled && !app.telemetry_stale() && !app.paused;

    // Output flags survive a lost socket by design. They are last-known
    // values, not proof that an output is still running now.
    if !usable && sampled {
        spans.extend(chrome::badge(
            if app.paused && !app.telemetry_stale() {
                "PAUSED"
            } else {
                "STALE"
            },
            theme.warning,
            theme,
        ));
        if snapshot.stream.active {
            spans.push(Span::styled(
                format!(
                    " last stream {}",
                    util::duration_ms(snapshot.stream.duration_ms)
                ),
                Style::default().fg(theme.muted),
            ));
        }
        if snapshot.record.active {
            spans.push(Span::styled(
                format!(
                    " last rec {}",
                    util::duration_ms(snapshot.record.duration_ms)
                ),
                Style::default().fg(theme.muted),
            ));
        }
        spans.push(dot(theme, app.unicode));
    }

    if usable && snapshot.stream.active {
        spans.extend(chrome::chip(
            if snapshot.stream.reconnecting {
                "RECONNECTING"
            } else {
                "LIVE"
            },
            if snapshot.stream.reconnecting {
                theme.warning
            } else {
                theme.danger
            },
            // Blink the badge roughly twice a second so it reads as live even
            // in a screenshot-still terminal.
            app.frame % 20 < 14,
            theme,
            app.unicode,
        ));
        spans.push(Span::styled(
            format!(" {}", util::duration_ms(snapshot.stream.duration_ms)),
            Style::default().fg(theme.fg),
        ));
        spans.push(dot(theme, app.unicode));
    }

    if usable && snapshot.record.active {
        spans.extend(chrome::chip(
            if snapshot.record.paused {
                "REC PAUSED"
            } else {
                "REC"
            },
            if snapshot.record.paused {
                theme.warning
            } else {
                theme.info
            },
            !snapshot.record.paused,
            theme,
            app.unicode,
        ));
        spans.push(Span::styled(
            format!(" {}", util::duration_ms(snapshot.record.duration_ms)),
            Style::default().fg(theme.fg),
        ));
        spans.push(dot(theme, app.unicode));
    }

    // Count of everything raised this session. The banner expires, but the
    // fact that frames were lost should not disappear with it.
    let mut alert_index = None;
    if app.alert_count > 0 {
        alert_index = Some(spans.len());
        spans.push(Span::styled(
            format!(
                "{} {}",
                if app.unicode { "▲" } else { "!" },
                app.alert_count
            ),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(dot(theme, app.unicode));
    }

    spans.extend(connection_chip(
        &snapshot.connection,
        snapshot,
        theme,
        app.unicode,
    ));
    (Line::from(spans), alert_index)
}

fn connection_chip(
    state: &ConnectionState,
    snapshot: &Snapshot,
    theme: &Theme,
    unicode: bool,
) -> Vec<Span<'static>> {
    let color = match state {
        ConnectionState::Connected if snapshot.polls == 0 => theme.warning,
        ConnectionState::Connected => theme.success,
        ConnectionState::Connecting { .. } => theme.warning,
        ConnectionState::Disconnected { .. } => theme.danger,
        ConnectionState::Fatal { .. } => theme.danger,
    };
    let mut spans = chrome::chip(&state.label(), color, true, theme, unicode);

    // Once connected, the poll counter is the honest "data is flowing" signal
    // — a green chip alone would survive a silent, wedged socket.
    if state.is_connected() {
        spans.push(Span::styled(
            if snapshot.polls == 0 {
                " WARMING UP".to_string()
            } else {
                format!(" #{}", snapshot.polls)
            },
            Style::default().fg(if snapshot.polls == 0 {
                theme.warning
            } else {
                theme.muted
            }),
        ));
    }
    spans
}

fn dot(theme: &Theme, unicode: bool) -> Span<'static> {
    Span::styled(
        if unicode { " · " } else { " | " },
        Style::default().fg(theme.border),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::obs::stats::{RecordStatus, StreamStatus, Version};
    use crate::tui::testing::line_text;
    use std::time::Duration;

    fn app() -> App {
        App::new(Config::default())
    }

    #[test]
    fn identity_always_shows_the_app_and_its_version() {
        let rendered = line_text(&identity(&app()));
        assert!(rendered.starts_with(APP_NAME));
        assert!(rendered.contains(crate::VERSION));
    }

    #[test]
    fn the_header_version_can_be_pinned() {
        // The screenshot generator relies on this to keep docs/images stable
        // across releases.
        let mut app = app();
        app.version = "9.9.9".into();
        let rendered = line_text(&identity(&app));
        assert!(rendered.contains("v9.9.9"));
        assert!(!rendered.contains(crate::VERSION));
    }

    #[test]
    fn identity_gains_the_obs_banner_once_it_is_known() {
        let mut app = app();
        assert!(!line_text(&identity(&app)).contains("OBS"));

        app.snapshot.version = Version {
            obs_version: "30.2.3".into(),
            websocket_version: "5.5.0".into(),
            platform: "linux".into(),
        };
        let rendered = line_text(&identity(&app));
        assert!(rendered.contains("OBS 30.2.3"));
        assert!(rendered.contains("(linux)"));
        assert!(rendered.contains("ws 5.5.0"));
    }

    #[test]
    fn the_connection_chip_reflects_each_state() {
        let mut app = app();
        assert!(line_text(&status(&app).0).contains("CONNECTING"));

        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.polls = 42;
        let connected = line_text(&status(&app).0);
        assert!(connected.contains("CONNECTED"));
        assert!(
            connected.contains("#42"),
            "poll count proves data is flowing"
        );

        app.snapshot.connection = ConnectionState::Disconnected {
            reason: "closed".into(),
            hint: None,
            retry_in: Duration::from_secs(2),
        };
        let offline = line_text(&status(&app).0);
        assert!(offline.contains("OFFLINE"));
        assert!(!offline.contains('#'), "no poll count while offline");
    }

    #[test]
    fn retry_attempts_are_numbered_in_the_chip() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Connecting { attempt: 3 };
        assert!(line_text(&status(&app).0).contains("RETRY #3"));
    }

    #[test]
    fn lost_connections_show_retained_outputs_as_stale_not_live() {
        for connection in [
            ConnectionState::Disconnected {
                reason: "closed".into(),
                hint: None,
                retry_in: Duration::from_secs(2),
            },
            ConnectionState::Connecting { attempt: 3 },
            ConnectionState::Fatal {
                reason: "auth".into(),
                hint: None,
            },
        ] {
            let mut app = app();
            app.snapshot.connection = connection;
            app.snapshot.polls = 42;
            app.snapshot.stream.active = true;
            app.snapshot.stream.duration_ms = 60_000;
            app.snapshot.record.active = true;
            let rendered = line_text(&status(&app).0);
            assert!(rendered.contains("STALE"), "{rendered}");
            assert!(rendered.contains("last stream 01:00"), "{rendered}");
            assert!(rendered.contains("last rec"), "{rendered}");
            assert!(!rendered.contains("LIVE"), "{rendered}");
            assert!(!rendered.contains("REC"), "{rendered}");
        }
    }

    #[test]
    fn connected_without_a_poll_is_warming_up_not_live() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.stream.active = true;
        app.snapshot.record.active = true;
        let rendered = line_text(&status(&app).0);
        assert!(rendered.contains("WARMING UP"), "{rendered}");
        assert!(!rendered.contains("LIVE"), "{rendered}");
        assert!(!rendered.contains("REC"), "{rendered}");
        assert!(!rendered.contains("#0"), "{rendered}");
    }

    #[test]
    fn a_live_stream_shows_a_badge_and_its_elapsed_time() {
        let mut app = app();
        app.snapshot.polls = 1;
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 3_723_000,
            ..StreamStatus::default()
        };
        let rendered = line_text(&status(&app).0);
        assert!(rendered.contains("LIVE"));
        assert!(rendered.contains("1:02:03"));
    }

    #[test]
    fn a_reconnecting_stream_says_so_instead_of_claiming_to_be_live() {
        let mut app = app();
        app.snapshot.polls = 1;
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.stream = StreamStatus {
            active: true,
            reconnecting: true,
            ..StreamStatus::default()
        };
        let rendered = line_text(&status(&app).0);
        assert!(rendered.contains("RECONNECTING"));
        assert!(!rendered.contains("● LIVE"));
    }

    #[test]
    fn recording_and_streaming_badges_coexist() {
        let mut app = app();
        app.snapshot.polls = 1;
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.stream = StreamStatus {
            active: true,
            ..StreamStatus::default()
        };
        app.snapshot.record = RecordStatus {
            active: true,
            ..RecordStatus::default()
        };
        let rendered = line_text(&status(&app).0);
        assert!(rendered.contains("LIVE"));
        assert!(rendered.contains("REC"));
    }

    #[test]
    fn a_paused_recording_is_labelled_as_paused() {
        let mut app = app();
        app.snapshot.polls = 1;
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.record = RecordStatus {
            active: true,
            paused: true,
            ..RecordStatus::default()
        };
        assert!(line_text(&status(&app).0).contains("REC PAUSED"));
    }

    #[test]
    fn raised_alerts_leave_a_counter_in_the_header() {
        let mut app = app();
        assert!(!line_text(&status(&app).0).contains('▲'));

        app.alert_count = 3;
        let rendered = line_text(&status(&app).0);
        assert!(rendered.contains("▲ 3"), "{rendered}");

        // And the ASCII form avoids the glyph.
        app.unicode = false;
        assert!(line_text(&status(&app).0).contains("! 3"));

        // The counter is the click target, so the span index has to come back
        // with it — and there must be nothing to click when nothing was
        // raised.
        assert!(status(&app).1.is_some(), "the counter must be clickable");
        app.alert_count = 0;
        assert_eq!(
            status(&app).1,
            None,
            "no counter drawn means no click target"
        );
    }

    #[test]
    fn idle_outputs_show_no_badges() {
        let app = app();
        let rendered = line_text(&status(&app).0);
        assert!(!rendered.contains("LIVE"));
        assert!(!rendered.contains("REC"));
    }
}

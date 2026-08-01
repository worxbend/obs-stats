//! The top bar: identity, connection state, live output badges.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::metrics::{ConnectionState, Snapshot};
use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::chrome;
use crate::{APP_NAME, util};

/// Height the header occupies, including its rule.
pub const HEIGHT: u16 = 2;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let [top, rule] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top);

    frame.render_widget(Paragraph::new(identity(app)), left);
    frame.render_widget(
        Paragraph::new(status(app)).alignment(Alignment::Right),
        right,
    );

    let glyphs = chrome::glyphs(app.unicode);
    let divider = if app.unicode { "─" } else { "-" };
    let _ = glyphs;
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            divider.repeat(rule.width as usize),
            Style::default().fg(app.theme.border),
        ))),
        rule,
    );
}

/// `obs-stats v0.1.0 · OBS 30.2.3 (linux) · ws 5.5.0`
fn identity(app: &App) -> Line<'static> {
    let theme = app.theme;
    let mut spans = vec![
        Span::styled(
            APP_NAME.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}", app.version),
            Style::default().fg(theme.muted),
        ),
    ];

    let version = &app.snapshot.version;
    if !version.obs_version.is_empty() {
        spans.push(dot(theme, app.unicode));
        spans.push(Span::styled(
            format!("OBS {}", version.obs_version),
            Style::default().fg(theme.fg),
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

/// Right-hand side: live badges, then the connection chip.
fn status(app: &App) -> Line<'static> {
    let theme = app.theme;
    let snapshot = &app.snapshot;
    let mut spans = Vec::new();

    if snapshot.stream.active {
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
            app.unicode,
        ));
        spans.push(Span::styled(
            format!(" {}", util::duration_ms(snapshot.stream.duration_ms)),
            Style::default().fg(theme.fg),
        ));
        spans.push(dot(theme, app.unicode));
    }

    if snapshot.record.active {
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
    if app.alert_count > 0 {
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
    Line::from(spans)
}

fn connection_chip(
    state: &ConnectionState,
    snapshot: &Snapshot,
    theme: &Theme,
    unicode: bool,
) -> Vec<Span<'static>> {
    let color = match state {
        ConnectionState::Connected => theme.success,
        ConnectionState::Connecting { .. } => theme.warning,
        ConnectionState::Disconnected { .. } => theme.danger,
        ConnectionState::Fatal { .. } => theme.danger,
    };
    let mut spans = chrome::chip(&state.label(), color, true, unicode);

    // Once connected, the poll counter is the honest "data is flowing" signal
    // — a green chip alone would survive a silent, wedged socket.
    if state.is_connected() {
        spans.push(Span::styled(
            format!(" #{}", snapshot.polls),
            Style::default().fg(theme.muted),
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
    use std::time::Duration;

    fn app() -> App {
        App::new(Config::default())
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn identity_always_shows_the_app_and_its_version() {
        let rendered = text(&identity(&app()));
        assert!(rendered.starts_with(APP_NAME));
        assert!(rendered.contains(crate::VERSION));
    }

    #[test]
    fn the_header_version_can_be_pinned() {
        // The screenshot generator relies on this to keep docs/images stable
        // across releases.
        let mut app = app();
        app.version = "9.9.9".into();
        let rendered = text(&identity(&app));
        assert!(rendered.contains("v9.9.9"));
        assert!(!rendered.contains(crate::VERSION));
    }

    #[test]
    fn identity_gains_the_obs_banner_once_it_is_known() {
        let mut app = app();
        assert!(!text(&identity(&app)).contains("OBS"));

        app.snapshot.version = Version {
            obs_version: "30.2.3".into(),
            websocket_version: "5.5.0".into(),
            platform: "linux".into(),
        };
        let rendered = text(&identity(&app));
        assert!(rendered.contains("OBS 30.2.3"));
        assert!(rendered.contains("(linux)"));
        assert!(rendered.contains("ws 5.5.0"));
    }

    #[test]
    fn the_connection_chip_reflects_each_state() {
        let mut app = app();
        assert!(text(&status(&app)).contains("CONNECTING"));

        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.polls = 42;
        let connected = text(&status(&app));
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
        let offline = text(&status(&app));
        assert!(offline.contains("OFFLINE"));
        assert!(!offline.contains('#'), "no poll count while offline");
    }

    #[test]
    fn retry_attempts_are_numbered_in_the_chip() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Connecting { attempt: 3 };
        assert!(text(&status(&app)).contains("RETRY #3"));
    }

    #[test]
    fn a_live_stream_shows_a_badge_and_its_elapsed_time() {
        let mut app = app();
        app.snapshot.connection = ConnectionState::Connected;
        app.snapshot.stream = StreamStatus {
            active: true,
            duration_ms: 3_723_000,
            ..StreamStatus::default()
        };
        let rendered = text(&status(&app));
        assert!(rendered.contains("LIVE"));
        assert!(rendered.contains("1:02:03"));
    }

    #[test]
    fn a_reconnecting_stream_says_so_instead_of_claiming_to_be_live() {
        let mut app = app();
        app.snapshot.stream = StreamStatus {
            active: true,
            reconnecting: true,
            ..StreamStatus::default()
        };
        let rendered = text(&status(&app));
        assert!(rendered.contains("RECONNECTING"));
        assert!(!rendered.contains("● LIVE"));
    }

    #[test]
    fn recording_and_streaming_badges_coexist() {
        let mut app = app();
        app.snapshot.stream = StreamStatus {
            active: true,
            ..StreamStatus::default()
        };
        app.snapshot.record = RecordStatus {
            active: true,
            ..RecordStatus::default()
        };
        let rendered = text(&status(&app));
        assert!(rendered.contains("LIVE"));
        assert!(rendered.contains("REC"));
    }

    #[test]
    fn a_paused_recording_is_labelled_as_paused() {
        let mut app = app();
        app.snapshot.record = RecordStatus {
            active: true,
            paused: true,
            ..RecordStatus::default()
        };
        assert!(text(&status(&app)).contains("REC PAUSED"));
    }

    #[test]
    fn raised_alerts_leave_a_counter_in_the_header() {
        let mut app = app();
        assert!(!text(&status(&app)).contains('▲'));

        app.alert_count = 3;
        let rendered = text(&status(&app));
        assert!(rendered.contains("▲ 3"), "{rendered}");

        // And the ASCII form avoids the glyph.
        app.unicode = false;
        assert!(text(&status(&app)).contains("! 3"));
    }

    #[test]
    fn idle_outputs_show_no_badges() {
        let app = app();
        let rendered = text(&status(&app));
        assert!(!rendered.contains("LIVE"));
        assert!(!rendered.contains("REC"));
    }
}

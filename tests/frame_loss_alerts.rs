//! End-to-end: an OBS poll reporting lost frames must reach the banner, the
//! log, the header counter, the screen, and the notifier.
//!
//! Drives the real types through the public API. The two ends this cannot
//! reach are OBS itself and the desktop's D-Bus daemon; both are exercised
//! separately (`obs-stats once` and `examples/notification_check`).

use std::time::{Duration, Instant};

use obs_stats::config::Config;
use obs_stats::metrics::AlertKind;
use obs_stats::notify::{Notification, Notifier};
use obs_stats::obs::client::{PollData, Update};
use obs_stats::obs::stats::{RecordStatus, Stats, StreamStatus};
use obs_stats::tui::app::App;
use obs_stats::tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A poll where the stream is live and has dropped `dropped` frames in total.
fn poll(dropped: u64, encoder_skipped: u64) -> Update {
    Update::Poll(Box::new(PollData {
        stats: Stats {
            active_fps: 60.0,
            output_skipped_frames: encoder_skipped,
            output_total_frames: 10_000,
            ..Stats::default()
        },
        stream: StreamStatus {
            active: true,
            skipped_frames: dropped,
            total_frames: 10_000,
            ..StreamStatus::default()
        },
        record: RecordStatus::default(),
    }))
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dropped_frames_surface_everywhere_they_should() {
    let mut app = App::new(Config::default());
    let start = Instant::now();

    // Baseline plus a clean poll: the dashboard should stay quiet.
    app.apply(poll(0, 0), start);
    let alerts = app.apply(poll(0, 0), start + Duration::from_secs(1));
    assert!(alerts.is_empty(), "a healthy stream must not alert");
    assert!(app.banner.is_none());
    assert!(!screen(&mut app).contains("DROPPING FRAMES"));

    // Now the uplink gives out.
    let alerts = app.apply(poll(9, 0), start + Duration::from_secs(2));
    assert_eq!(
        alerts.len(),
        1,
        "sustained loss should raise exactly one alert"
    );
    assert_eq!(alerts[0].kind, AlertKind::NetworkDropped);
    assert_eq!(alerts[0].frames, 9);

    // 1. The banner.
    let banner = app.banner.as_ref().expect("a banner should be raised");
    assert_eq!(banner.alert.kind, AlertKind::NetworkDropped);

    // 2. On screen, from whichever view happens to be open.
    let rendered = screen(&mut app);
    assert!(rendered.contains("DROPPING FRAMES"), "{rendered}");
    assert!(rendered.contains("viewers are seeing stutter"));

    // 3. The header counter.
    assert_eq!(app.alert_count, 1);
    assert!(rendered.contains("▲ 1"), "header should carry the count");

    // 4. The log, which outlives the banner.
    let logged = app.snapshot.log.last().expect("the alert should be logged");
    assert!(
        logged.message.contains("dropping frames"),
        "{}",
        logged.message
    );

    // 5. What the desktop would have been told.
    let notification = Notification::from_alert(&alerts[0]);
    assert_eq!(notification.summary, "OBS: dropping frames");
    assert!(notification.body.contains("9 frames"));
    assert!(notification.critical, "viewers seeing stutter is critical");

    // A disabled notifier must swallow it without complaint.
    Notifier::disabled().send(&alerts[0]);
}

#[test]
fn the_banner_expires_but_the_evidence_does_not() {
    let mut app = App::new(Config::default());
    let start = Instant::now();

    app.apply(poll(0, 0), start);
    app.apply(poll(9, 0), start + Duration::from_secs(1));
    assert!(app.banner.is_some());

    // Long after the banner's lifetime.
    let later = start + Duration::from_secs(600);
    app.tick(later);
    assert!(app.banner.is_none(), "the banner should clear itself");

    // But the count and the log line remain.
    assert_eq!(app.alert_count, 1);
    assert!(
        app.snapshot
            .log
            .iter()
            .any(|entry| entry.message.contains("dropping frames")),
        "the log must keep the record"
    );

    app.view = obs_stats::tui::view::View::Log;
    assert!(screen(&mut app).contains("dropping frames"));
}

#[test]
fn a_worse_alert_replaces_a_lesser_banner_but_not_the_reverse() {
    // No cooldown, so the encoder can speak again immediately and the test is
    // about banner precedence rather than about rate limiting.
    let mut config = Config::default();
    config.alerts.cooldown_seconds = 0;
    let mut app = App::new(config);
    let start = Instant::now();

    // Encoder trouble first: a warning.
    app.apply(poll(0, 0), start);
    app.apply(poll(0, 9), start + Duration::from_secs(1));
    assert_eq!(
        app.banner.as_ref().unwrap().alert.kind,
        AlertKind::EncoderSkipped
    );

    // Then the network gives out, which viewers actually see.
    app.apply(poll(9, 9), start + Duration::from_secs(2));
    assert_eq!(
        app.banner.as_ref().unwrap().alert.kind,
        AlertKind::NetworkDropped,
        "the critical alert should take the banner"
    );

    // More encoder trouble, while the critical banner is still showing.
    app.apply(poll(9, 40), start + Duration::from_secs(3));
    assert_eq!(
        app.banner.as_ref().unwrap().alert.kind,
        AlertKind::NetworkDropped,
        "a warning must not bury a critical banner that is still up"
    );

    // Once it has expired, a later warning is free to take the row.
    let after = start + Duration::from_secs(120);
    app.tick(after);
    app.apply(poll(9, 80), after);
    assert_eq!(
        app.banner.as_ref().unwrap().alert.kind,
        AlertKind::EncoderSkipped,
        "an expired critical banner should not block later alerts forever"
    );
}

#[test]
fn turning_alerts_off_keeps_the_dashboard_silent() {
    let mut config = Config::default();
    config.alerts.enabled = false;
    let mut app = App::new(config);
    let start = Instant::now();

    app.apply(poll(0, 0), start);
    let alerts = app.apply(poll(500, 500), start + Duration::from_secs(1));

    assert!(alerts.is_empty());
    assert!(app.banner.is_none());
    assert_eq!(app.alert_count, 0);
    assert!(!screen(&mut app).contains("DROPPING FRAMES"));
}

#[test]
fn a_paused_dashboard_raises_no_alerts() {
    // Pausing freezes the metrics, so there is nothing new to alert on.
    let mut app = App::new(Config::default());
    let start = Instant::now();
    app.apply(poll(0, 0), start);
    app.paused = true;

    let alerts = app.apply(poll(500, 0), start + Duration::from_secs(1));
    assert!(alerts.is_empty());
    assert!(app.banner.is_none());
}

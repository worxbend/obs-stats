//! Desktop notifications for frame-loss alerts.
//!
//! Notifications go out on a dedicated thread. Showing one is a synchronous
//! D-Bus round trip, and a desktop whose notification daemon has wedged would
//! otherwise stall the render loop — the one thing the whole architecture is
//! arranged to prevent. The UI hands an alert over and forgets about it.
//!
//! On Linux this speaks D-Bus directly through a pure-Rust client, so there is
//! no C library to link and nothing to install. Everywhere else it compiles to
//! a no-op, because the alert is already visible in the TUI.

use std::sync::mpsc;

use crate::metrics::alerts::{Alert, Severity};

/// How many consecutive delivery failures before giving up for the session.
///
/// A missing or broken notification daemon fails on every attempt; retrying
/// forever would burn a thread and log a line per alert for no benefit.
const MAX_FAILURES: u32 = 3;

/// What gets shown, independent of any platform API.
///
/// Split out so the wording and urgency mapping can be tested without a
/// desktop session — which CI does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    pub critical: bool,
    /// Milliseconds to remain on screen. `0` means "until dismissed".
    pub timeout_ms: u32,
}

impl Notification {
    pub fn from_alert(alert: &Alert) -> Self {
        let critical = alert.severity() == Severity::Critical;
        Self {
            summary: alert.title(),
            body: format!("{}\n{}", alert.summary(), alert.kind.hint()),
            critical,
            // A dropped stream should stay up until acknowledged; a render
            // hiccup should not follow you around.
            timeout_ms: if critical { 0 } else { 8_000 },
        }
    }
}

/// Handle for sending notifications.
///
/// Cloning is cheap and every clone feeds the same worker.
#[derive(Debug, Clone)]
pub struct Notifier {
    sender: Option<mpsc::Sender<Notification>>,
}

impl Notifier {
    /// A notifier that discards everything. Used when notifications are turned
    /// off, and on platforms with no backend.
    pub fn disabled() -> Self {
        Self { sender: None }
    }

    /// Start the worker thread, unless `enabled` is false.
    pub fn spawn(enabled: bool) -> Self {
        if !enabled || !backend::available() {
            return Self::disabled();
        }

        let (sender, receiver) = mpsc::channel::<Notification>();
        let spawned = std::thread::Builder::new()
            .name("obs-stats-notify".to_string())
            .spawn(move || worker(&receiver));

        match spawned {
            Ok(_) => Self {
                sender: Some(sender),
            },
            Err(error) => {
                tracing::warn!(%error, "could not start the notification thread");
                Self::disabled()
            }
        }
    }

    /// Whether anything will actually be delivered.
    pub fn is_enabled(&self) -> bool {
        self.sender.is_some()
    }

    /// Queue a notification for `alert`. Never blocks, never fails loudly.
    pub fn send(&self, alert: &Alert) {
        let Some(sender) = &self.sender else { return };
        // A closed channel means the worker gave up; dropping is correct.
        let _ = sender.send(Notification::from_alert(alert));
    }
}

/// Drain the queue until the sender goes away or the desktop stops answering.
fn worker(receiver: &mpsc::Receiver<Notification>) {
    let mut failures = 0_u32;

    while let Ok(notification) = receiver.recv() {
        match backend::show(&notification) {
            Ok(()) => failures = 0,
            Err(error) => {
                failures += 1;
                tracing::debug!(%error, failures, "notification delivery failed");
                if failures >= MAX_FAILURES {
                    tracing::warn!(
                        "giving up on desktop notifications after {failures} failures; \
                         alerts are still shown in the dashboard"
                    );
                    return;
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::Notification;

    /// Notifications need a session bus. Without one — a bare TTY, a container,
    /// an ssh session — there is nothing to talk to, so skip the thread
    /// entirely rather than failing three times to discover it.
    pub fn available() -> bool {
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
            || std::env::var_os("XDG_RUNTIME_DIR").is_some()
    }

    pub fn show(notification: &Notification) -> Result<(), String> {
        use notify_rust::{Hint, Notification as Desktop, Timeout, Urgency};

        let mut desktop = Desktop::new();
        desktop
            .appname("obs-stats")
            .summary(&notification.summary)
            .body(&notification.body)
            // Matches the freedesktop category for a hardware/system problem,
            // which is how notification daemons decide on styling and sound.
            .hint(Hint::Category("device.error".to_string()))
            .hint(Hint::Urgency(if notification.critical {
                Urgency::Critical
            } else {
                Urgency::Normal
            }))
            .icon("video-display")
            .timeout(if notification.timeout_ms == 0 {
                Timeout::Never
            } else {
                Timeout::Milliseconds(notification.timeout_ms)
            });

        desktop
            .show()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(target_os = "linux"))]
mod backend {
    use super::Notification;

    pub fn available() -> bool {
        false
    }

    pub fn show(_notification: &Notification) -> Result<(), String> {
        Err("desktop notifications are only implemented for Linux".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::alerts::AlertKind;
    use std::time::{Duration, Instant};

    fn alert(kind: AlertKind, frames: u64) -> Alert {
        Alert {
            kind,
            frames,
            window: Duration::from_secs(10),
            at: Instant::now(),
        }
    }

    #[test]
    fn a_notification_carries_the_cause_and_the_fix() {
        let notification = Notification::from_alert(&alert(AlertKind::NetworkDropped, 12));
        assert_eq!(notification.summary, "OBS: dropping frames");
        assert!(notification.body.contains("12 frames"));
        assert!(notification.body.contains("viewers are seeing stutter"));
        // The point of the notification is knowing what to go and do.
        assert!(
            notification.body.contains("Lower the bitrate"),
            "{}",
            notification.body
        );
    }

    #[test]
    fn critical_alerts_stay_on_screen_until_dismissed() {
        let dropped = Notification::from_alert(&alert(AlertKind::NetworkDropped, 5));
        assert!(dropped.critical);
        assert_eq!(dropped.timeout_ms, 0, "0 means 'until dismissed'");

        let reconnect = Notification::from_alert(&alert(AlertKind::Reconnecting, 0));
        assert!(reconnect.critical);
    }

    #[test]
    fn warnings_expire_on_their_own() {
        for kind in [AlertKind::RendererSkipped, AlertKind::EncoderSkipped] {
            let notification = Notification::from_alert(&alert(kind, 4));
            assert!(!notification.critical, "{kind:?} should not be critical");
            assert!(notification.timeout_ms > 0, "{kind:?} should expire");
        }
    }

    #[test]
    fn every_alert_kind_produces_a_complete_notification() {
        for kind in AlertKind::ALL {
            let notification = Notification::from_alert(&alert(kind, 7));
            assert!(!notification.summary.is_empty(), "{kind:?}");
            // Summary line plus hint line.
            assert!(
                notification.body.contains('\n'),
                "{kind:?} has no hint line"
            );
            assert!(
                notification.body.len() > notification.summary.len(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_disabled_notifier_accepts_and_drops_everything() {
        let notifier = Notifier::disabled();
        assert!(!notifier.is_enabled());
        // Must not panic or block.
        notifier.send(&alert(AlertKind::NetworkDropped, 3));
    }

    #[test]
    fn spawning_with_notifications_off_yields_a_disabled_notifier() {
        assert!(!Notifier::spawn(false).is_enabled());
    }

    #[test]
    fn a_notifier_can_be_cloned_and_shared() {
        let notifier = Notifier::disabled();
        let clone = notifier.clone();
        clone.send(&alert(AlertKind::EncoderSkipped, 9));
        assert_eq!(notifier.is_enabled(), clone.is_enabled());
    }

    #[test]
    fn the_worker_stops_after_repeated_failures() {
        // On CI there is no notification daemon, so `show` fails every time.
        // The worker must terminate rather than spin.
        let (sender, receiver) = mpsc::channel();
        for _ in 0..(MAX_FAILURES + 5) {
            sender
                .send(Notification::from_alert(&alert(
                    AlertKind::NetworkDropped,
                    1,
                )))
                .unwrap();
        }
        drop(sender);

        let handle = std::thread::spawn(move || worker(&receiver));
        // Joining proves it returned instead of blocking forever.
        handle.join().expect("worker should exit cleanly");
    }
}

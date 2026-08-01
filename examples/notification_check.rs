//! Send one test notification of each severity and report what happened.
//!
//! For answering "why am I not getting notifications?" without having to make
//! OBS actually drop frames:
//!
//! ```text
//! cargo run --example notification_check
//! ```
//!
//! It reports whether a session bus was found and whether the notifier
//! started, then sends a warning and a critical alert a second apart.

use std::time::{Duration, Instant};

use obs_stats::metrics::alerts::{Alert, AlertKind};
use obs_stats::notify::{Notification, Notifier};

fn main() {
    println!("obs-stats notification check\n");

    for (name, value) in [
        (
            "DBUS_SESSION_BUS_ADDRESS",
            std::env::var("DBUS_SESSION_BUS_ADDRESS"),
        ),
        ("XDG_RUNTIME_DIR", std::env::var("XDG_RUNTIME_DIR")),
    ] {
        match value {
            Ok(value) => println!("  {name} = {value}"),
            Err(_) => println!("  {name} is unset"),
        }
    }

    let notifier = Notifier::spawn(true);
    println!("\n  notifier enabled: {}", notifier.is_enabled());

    if !notifier.is_enabled() {
        println!(
            "\nNo session bus was found, so notifications are off. This is normal on a bare\n\
             TTY, over ssh, or inside a container. Alerts still appear in the dashboard."
        );
        return;
    }

    for (kind, frames) in [
        (AlertKind::RendererSkipped, 5),
        (AlertKind::NetworkDropped, 12),
    ] {
        let alert = Alert {
            kind,
            frames,
            window: Duration::from_secs(10),
            at: Instant::now(),
        };
        let preview = Notification::from_alert(&alert);
        println!(
            "\n  sending: {}\n           {}",
            preview.summary,
            preview.body.replace('\n', "\n           ")
        );
        notifier.send(&alert);
        std::thread::sleep(Duration::from_secs(1));
    }

    // The worker delivers asynchronously; give it a moment before exiting.
    std::thread::sleep(Duration::from_millis(500));
    println!(
        "\nIf nothing appeared, check your desktop's notification daemon and its\ndo-not-disturb setting."
    );
}

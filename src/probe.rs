//! One-shot data collection.
//!
//! Drives a session just long enough to fill a [`Snapshot`], then shuts it
//! down. Used by `obs-stats once` and by the screenshot generator, so both
//! see exactly the data the dashboard would.

use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::metrics::Snapshot;
use crate::obs::client::{self, ConnectionParams, Update};

/// How long to keep listening after the core picture is complete.
///
/// The mute/volume probe is only issued *after* the input list arrives, so
/// without this grace period every fader position would read as unknown.
pub const AUDIO_GRACE: Duration = Duration::from_millis(400);

/// Connect, collect one full picture, and disconnect.
///
/// Returns as soon as a hot poll, an inventory, and every input's fader
/// position have landed — or after [`AUDIO_GRACE`] with whatever arrived.
pub async fn collect(params: ConnectionParams, timeout: Duration) -> Result<Snapshot> {
    let (mut updates, commands) = client::spawn(params);
    let result = drain(&mut updates, timeout).await;
    let _ = commands.send(client::Command::Shutdown).await;
    result
}

async fn drain(
    updates: &mut tokio::sync::mpsc::Receiver<Update>,
    timeout: Duration,
) -> Result<Snapshot> {
    let mut snapshot = Snapshot::new(8);
    let mut have_poll = false;
    let mut have_inventory = false;
    // Set once the core picture is complete; from then on we drain whatever
    // else arrives until this shorter deadline expires.
    let mut grace_deadline: Option<tokio::time::Instant> = None;

    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let until = grace_deadline.unwrap_or(deadline);
        let update = tokio::select! {
            update = updates.recv() => update,
            _ = tokio::time::sleep_until(until) => {
                // Expiring during the grace period is success: we have the
                // core picture and simply stopped waiting for the extras.
                if grace_deadline.is_some() {
                    return Ok(snapshot);
                }
                return Err(Error::RequestTimeout {
                    request: "GetStats".to_string(),
                    timeout_ms: timeout.as_millis() as u64,
                });
            }
        };

        let Some(update) = update else {
            return Err(Error::Disconnected);
        };

        // A failure that will never succeed should not wait out the timeout.
        if let Update::Fatal { reason, hint } = &update {
            return Err(Error::ConfigInvalid(match hint {
                Some(hint) => format!("{reason} ({hint})"),
                None => reason.clone(),
            }));
        }

        match &update {
            Update::Poll(_) => have_poll = true,
            Update::Inventory(_) => have_inventory = true,
            _ => {}
        }
        snapshot.apply(update, Instant::now());

        if grace_deadline.is_none() && have_poll && have_inventory {
            // Nothing to wait for when OBS reports no audio inputs.
            if snapshot.inputs.is_empty() {
                return Ok(snapshot);
            }
            grace_deadline = Some(tokio::time::Instant::now() + AUDIO_GRACE);
        }

        // Stop early once every input has reported a fader position.
        if grace_deadline.is_some()
            && snapshot
                .inputs
                .iter()
                .all(|input| input.volume_db.is_finite())
        {
            return Ok(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obs::client::{Inventory, PollData};
    use crate::obs::stats::AudioInput;

    fn channel() -> (
        tokio::sync::mpsc::Sender<Update>,
        tokio::sync::mpsc::Receiver<Update>,
    ) {
        tokio::sync::mpsc::channel(32)
    }

    #[tokio::test]
    async fn returns_once_poll_and_inventory_have_landed() {
        let (tx, mut rx) = channel();
        tx.send(Update::Poll(Box::default())).await.unwrap();
        tx.send(Update::Inventory(Box::default())).await.unwrap();

        let snapshot = drain(&mut rx, Duration::from_secs(5)).await.unwrap();
        assert_eq!(snapshot.polls, 1);
    }

    #[tokio::test]
    async fn waits_for_fader_positions_when_inputs_exist() {
        let (tx, mut rx) = channel();
        tx.send(Update::Poll(Box::default())).await.unwrap();
        tx.send(Update::Inventory(Box::new(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                volume_db: f64::NEG_INFINITY,
                ..AudioInput::default()
            }],
            ..Inventory::default()
        })))
        .await
        .unwrap();
        // The probe arrives as a two-slot mute/volume payload.
        tx.send(Update::Meters(vec![("Mic".to_string(), vec![0.0, -6.0])]))
            .await
            .unwrap();

        let snapshot = drain(&mut rx, Duration::from_secs(5)).await.unwrap();
        assert!((snapshot.inputs[0].volume_db + 6.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn gives_up_on_faders_after_the_grace_period() {
        let (tx, mut rx) = channel();
        tx.send(Update::Poll(Box::default())).await.unwrap();
        tx.send(Update::Inventory(Box::new(Inventory {
            inputs: vec![AudioInput {
                name: "Mic".into(),
                volume_db: f64::NEG_INFINITY,
                ..AudioInput::default()
            }],
            ..Inventory::default()
        })))
        .await
        .unwrap();
        // No probe is ever sent; the grace period must still let us finish.
        let snapshot = drain(&mut rx, Duration::from_secs(30)).await.unwrap();
        assert_eq!(snapshot.inputs.len(), 1);
        assert!(!snapshot.inputs[0].volume_db.is_finite());
    }

    #[tokio::test]
    async fn a_fatal_failure_returns_immediately_rather_than_timing_out() {
        let (tx, mut rx) = channel();
        tx.send(Update::Fatal {
            reason: "authentication failed".into(),
            hint: Some("Check the password"),
        })
        .await
        .unwrap();

        let error = drain(&mut rx, Duration::from_secs(60)).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("authentication failed"), "{message}");
        assert!(message.contains("Check the password"), "{message}");
    }

    #[tokio::test]
    async fn an_incomplete_picture_times_out() {
        let (tx, mut rx) = channel();
        // A poll but never an inventory.
        tx.send(Update::Poll(Box::default())).await.unwrap();
        drop(tx);

        // The sender is dropped, so this surfaces as a disconnect.
        let error = drain(&mut rx, Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(error, Error::Disconnected));
    }

    #[tokio::test]
    async fn a_silent_session_times_out_with_a_request_timeout() {
        let (_tx, mut rx) = channel();
        let error = drain(&mut rx, Duration::from_millis(50)).await.unwrap_err();
        assert!(matches!(error, Error::RequestTimeout { .. }));
    }

    #[tokio::test]
    async fn no_audio_inputs_means_no_waiting() {
        let (tx, mut rx) = channel();
        tx.send(Update::Poll(Box::default())).await.unwrap();
        tx.send(Update::Inventory(Box::new(Inventory {
            inputs: Vec::new(),
            ..Inventory::default()
        })))
        .await
        .unwrap();
        drop(tx);

        // A 30s budget that returns promptly proves the grace period was skipped.
        let snapshot = drain(&mut rx, Duration::from_secs(30)).await.unwrap();
        assert!(snapshot.inputs.is_empty());
    }

    #[tokio::test]
    async fn poll_data_reaches_the_snapshot() {
        use crate::obs::stats::Stats;

        let (tx, mut rx) = channel();
        tx.send(Update::Poll(Box::new(PollData {
            stats: Stats {
                cpu_usage: 9.5,
                ..Stats::default()
            },
            ..PollData::default()
        })))
        .await
        .unwrap();
        tx.send(Update::Inventory(Box::default())).await.unwrap();

        let snapshot = drain(&mut rx, Duration::from_secs(5)).await.unwrap();
        assert_eq!(snapshot.stats.cpu_usage, 9.5);
    }
}

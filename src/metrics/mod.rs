//! Turning OBS responses into something a dashboard can draw.
//!
//! [`Snapshot`] is the single model the views read; [`History`] is the ring
//! buffer behind every sparkline and chart. Neither performs I/O, so the whole
//! layer is exercised in tests by feeding it synthetic polls.

pub mod history;
pub mod snapshot;

pub use history::History;
pub use snapshot::{ConnectionState, Health, Histories, LogEntry, Snapshot};

/// How many samples cover `seconds` of history at a given poll cadence.
///
/// Always at least two, since a chart needs a segment to draw.
pub fn history_capacity(history_seconds: u64, poll_interval_ms: u64) -> usize {
    let interval = poll_interval_ms.max(1);
    ((history_seconds * 1000) / interval).max(2) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_covers_the_requested_window() {
        assert_eq!(history_capacity(120, 1000), 120);
        assert_eq!(history_capacity(120, 500), 240);
        assert_eq!(history_capacity(60, 250), 240);
    }

    #[test]
    fn capacity_never_collapses_below_a_drawable_series() {
        assert_eq!(history_capacity(10, 60_000), 2);
        assert_eq!(history_capacity(0, 1000), 2);
        // A zero interval must not divide by zero.
        assert_eq!(history_capacity(10, 0), 10_000);
    }
}

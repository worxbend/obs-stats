//! A fixed-capacity ring buffer of samples, sized to the retention window.
//!
//! Charts want the newest sample on the right and a stable capacity, so this
//! is a plain `VecDeque` with an eviction rule rather than anything clever.
//! Every accessor is O(n) at worst over a bounded n (a few hundred points).

use std::collections::VecDeque;

/// Rolling samples for one metric.
#[derive(Debug, Clone)]
pub struct History {
    samples: VecDeque<f64>,
    capacity: usize,
}

impl History {
    /// Create a history retaining at most `capacity` samples.
    ///
    /// A zero capacity is coerced to 1, so `latest()` always has somewhere to
    /// live and callers need not special-case it.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append a sample, evicting the oldest once full.
    ///
    /// Non-finite samples are recorded as `0.0`: a single NaN would otherwise
    /// poison `max()` and blank the whole chart.
    pub fn push(&mut self, value: f64) {
        let value = if value.is_finite() { value } else { 0.0 };
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    /// Resize the retention window, keeping the newest samples.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The most recent sample, or `0.0` when empty.
    pub fn latest(&self) -> f64 {
        self.samples.back().copied().unwrap_or(0.0)
    }

    pub fn max(&self) -> f64 {
        self.samples.iter().copied().fold(0.0_f64, f64::max)
    }

    pub fn min(&self) -> f64 {
        self.samples
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
            .min(f64::INFINITY)
    }

    pub fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().sum::<f64>() / self.samples.len() as f64
    }

    /// Oldest-to-newest iterator.
    pub fn iter(&self) -> impl Iterator<Item = f64> + '_ {
        self.samples.iter().copied()
    }

    /// The newest `count` samples, oldest first — what a chart of `count`
    /// columns should draw.
    pub fn tail(&self, count: usize) -> Vec<f64> {
        let skip = self.samples.len().saturating_sub(count);
        self.samples.iter().skip(skip).copied().collect()
    }

    /// The newest `count` samples as `u64`, for [`ratatui::widgets::Sparkline`].
    ///
    /// Negative samples clamp to zero rather than wrapping on cast.
    pub fn tail_u64(&self, count: usize) -> Vec<u64> {
        self.tail(count)
            .into_iter()
            .map(|value| value.max(0.0).round() as u64)
            .collect()
    }

    /// The newest `count` samples as `(x, y)` pairs for
    /// [`ratatui::widgets::Chart`], with `x` running `0..count`.
    pub fn tail_points(&self, count: usize) -> Vec<(f64, f64)> {
        self.tail(count)
            .into_iter()
            .enumerate()
            .map(|(index, value)| (index as f64, value))
            .collect()
    }

    /// An upper bound for a chart's y-axis: the observed max with 15%
    /// headroom, never below `floor`, so a flat-zero series still renders a
    /// sensible axis instead of collapsing to a single line.
    pub fn axis_max(&self, floor: f64) -> f64 {
        (self.max() * 1.15).max(floor)
    }

    /// The `(min, max)` a chart should plot between.
    ///
    /// A series that lives in a narrow band well above zero — 59–60 fps, or a
    /// 5–6 Mbps stream — is illegible on a zero-anchored axis: every sample
    /// lands in the top few rows and the variation that matters is invisible.
    /// So the floor lifts to just under the observed minimum once the whole
    /// series sits in the upper part of the range, and stays at zero
    /// otherwise, where "how close to nothing is this?" is the real question.
    pub fn axis_range(&self, floor: f64) -> (f64, f64) {
        let max = self.axis_max(floor);
        if self.is_empty() {
            return (0.0, max);
        }

        let min = self.min();
        // Only window in when the data occupies the top third of the axis;
        // below that, zero is genuinely informative.
        if min > max * 0.35 {
            let padded = min - (max - min).max(max * 0.05) * 0.35;
            (padded.max(0.0), max)
        } else {
            (0.0, max)
        }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(capacity: usize, values: &[f64]) -> History {
        let mut history = History::new(capacity);
        for value in values {
            history.push(*value);
        }
        history
    }

    #[test]
    fn evicts_the_oldest_sample_when_full() {
        let history = filled(3, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(history.len(), 3);
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
        assert_eq!(history.latest(), 4.0);
    }

    #[test]
    fn an_empty_history_answers_with_zeroes() {
        let history = History::new(8);
        assert!(history.is_empty());
        assert_eq!(history.latest(), 0.0);
        assert_eq!(history.max(), 0.0);
        assert_eq!(history.mean(), 0.0);
        assert!(history.tail(4).is_empty());
    }

    #[test]
    fn zero_capacity_is_coerced_to_one() {
        let history = filled(0, &[1.0, 2.0]);
        assert_eq!(history.capacity(), 1);
        assert_eq!(history.len(), 1);
        assert_eq!(history.latest(), 2.0);
    }

    #[test]
    fn non_finite_samples_are_stored_as_zero() {
        let history = filled(4, &[1.0, f64::NAN, f64::INFINITY, 2.0]);
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![1.0, 0.0, 0.0, 2.0]);
        // A NaN would otherwise make max() unusable.
        assert_eq!(history.max(), 2.0);
    }

    #[test]
    fn shrinking_the_capacity_keeps_the_newest_samples() {
        let mut history = filled(5, &[1.0, 2.0, 3.0, 4.0, 5.0]);
        history.set_capacity(2);
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![4.0, 5.0]);

        history.set_capacity(4);
        history.push(6.0);
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![4.0, 5.0, 6.0]);
    }

    #[test]
    fn tail_returns_at_most_what_exists() {
        let history = filled(10, &[1.0, 2.0, 3.0]);
        assert_eq!(history.tail(2), vec![2.0, 3.0]);
        assert_eq!(history.tail(10), vec![1.0, 2.0, 3.0]);
        assert_eq!(history.tail(0), Vec::<f64>::new());
    }

    #[test]
    fn tail_u64_rounds_and_clamps_negatives() {
        let history = filled(4, &[-3.0, 0.4, 0.6, 9.5]);
        assert_eq!(history.tail_u64(4), vec![0, 0, 1, 10]);
    }

    #[test]
    fn tail_points_indexes_from_zero() {
        let history = filled(4, &[5.0, 6.0]);
        assert_eq!(history.tail_points(4), vec![(0.0, 5.0), (1.0, 6.0)]);
    }

    #[test]
    fn axis_range_windows_in_on_a_narrow_high_band() {
        // 59–60 fps on a 0-anchored axis would be a flat line in the top rows.
        let history = filled(8, &[59.0, 59.5, 60.0, 59.2]);
        let (min, max) = history.axis_range(60.0);
        assert!(min > 50.0, "expected a lifted floor, got {min}");
        assert!(min < 59.0, "the lowest sample must stay on the chart");
        assert!(max >= 60.0);
    }

    #[test]
    fn axis_range_stays_anchored_at_zero_for_low_data() {
        // Congestion hovering near zero should read as "near zero".
        let history = filled(8, &[0.0, 2.0, 1.0, 4.0]);
        assert_eq!(history.axis_range(100.0).0, 0.0);

        // And a series spanning most of its range keeps the zero anchor too.
        let spanning = filled(8, &[5.0, 90.0, 40.0]);
        assert_eq!(spanning.axis_range(0.0).0, 0.0);
    }

    #[test]
    fn axis_range_handles_an_empty_or_flat_series() {
        assert_eq!(History::new(4).axis_range(60.0), (0.0, 60.0));

        // A perfectly flat series must still produce min < max.
        let flat = filled(4, &[42.0, 42.0, 42.0]);
        let (min, max) = flat.axis_range(0.0);
        assert!(min < max, "a flat series must not collapse the axis");
        assert!(min < 42.0 && max > 42.0);
    }

    #[test]
    fn axis_range_never_goes_negative() {
        let history = filled(4, &[0.1, 0.2, 0.15]);
        assert!(history.axis_range(0.0).0 >= 0.0);
    }

    #[test]
    fn axis_max_adds_headroom_and_respects_the_floor() {
        let history = filled(4, &[10.0, 20.0]);
        assert!((history.axis_max(0.0) - 23.0).abs() < 1e-9);
        // A flat-zero series must still produce a usable axis.
        assert_eq!(History::new(4).axis_max(60.0), 60.0);
    }

    #[test]
    fn statistics_over_a_partial_buffer() {
        let history = filled(10, &[2.0, 4.0, 6.0]);
        assert_eq!(history.min(), 2.0);
        assert_eq!(history.max(), 6.0);
        assert_eq!(history.mean(), 4.0);
    }

    #[test]
    fn clear_empties_but_keeps_the_capacity() {
        let mut history = filled(4, &[1.0, 2.0]);
        history.clear();
        assert!(history.is_empty());
        assert_eq!(history.capacity(), 4);
    }
}

//! Frame loss as one value, so every panel reports it the same way.
//!
//! Three different things in OBS can lose a frame, and the dashboard shows
//! all three in four places:
//!
//! - the renderer skipping a frame it could not draw in time,
//! - the encoder skipping a frame it could not compress in time,
//! - the network dropping a frame the viewer will never see.
//!
//! Each is a count, a rate, and a pair of thresholds saying when that rate
//! stops being acceptable. Those three facts were re-derived at every site,
//! which meant the format string `"{count} ({percent})"` was written five
//! times and the threshold pairs — `0.5/2.0`, `0.5/1.0`, `0.1/1.0` — were
//! written twice each as bare literals with nothing naming them.
//!
//! This lives in the `tui` layer rather than in `metrics` on purpose: a
//! threshold that exists to pick a colour is a presentation decision, and
//! `Loss::color` needs a [`Theme`]. The counts and rates it is built from
//! stay where they were.

use crate::metrics::Snapshot;
use crate::obs::stats::StreamStatus;
use crate::tui::theme::Theme;
use crate::util;

/// One kind of frame loss, with the thresholds that colour it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loss {
    /// Frames lost.
    pub count: u64,
    /// Those frames as a percentage of the total.
    pub percent: f64,
    warn: f64,
    danger: f64,
}

impl Loss {
    /// Frames the renderer skipped over the current stream.
    ///
    /// The most forgiving thresholds of the three: a skipped render is a
    /// dropped frame the viewer never sees, so it matters least.
    pub fn render_skip(snapshot: &Snapshot) -> Self {
        Self {
            count: snapshot.session_render_skipped(),
            percent: snapshot.session_render_skip_percent(),
            warn: 0.5,
            danger: 2.0,
        }
    }

    /// Frames the encoder skipped over the current stream.
    pub fn output_skip(snapshot: &Snapshot) -> Self {
        Self {
            count: snapshot.session_output_skipped(),
            percent: snapshot.session_output_skip_percent(),
            warn: 0.5,
            danger: 1.0,
        }
    }

    /// Frames the network dropped: the strictest thresholds, because these
    /// are the ones a viewer actually sees as a stutter.
    pub fn network_drop(stream: &StreamStatus) -> Self {
        Self {
            count: stream.skipped_frames,
            percent: stream.drop_percent(),
            warn: 0.1,
            danger: 1.0,
        }
    }

    /// `"{count} ({percent})"`, the form every panel prints.
    pub fn text(self) -> String {
        format!("{} ({})", self.count, util::percent(self.percent))
    }

    /// Green, amber or red for this rate against its own thresholds.
    pub fn color(self, theme: &Theme) -> ratatui::style::Color {
        theme.color_when_high(self.percent, self.warn, self.danger)
    }

    /// The rate as a `0.0..=1.0` fraction, for gauge fills.
    pub fn ratio(self) -> f64 {
        self.percent / 100.0
    }
}

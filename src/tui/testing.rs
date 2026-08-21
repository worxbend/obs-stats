//! Test-only helpers shared by the view and widget rendering tests.
//!
//! Rendering tests overwhelmingly want to assert on *what a widget says*, not
//! on how it was styled. Ratatui models a rendered row as a [`Line`] of
//! styled [`Span`](ratatui::text::Span)s, so getting at the words means
//! flattening the spans back into one string. Every view and widget module
//! had grown its own private copy of that flattening function; these are the
//! two shapes those copies took, written once.

use ratatui::text::Line;

/// Flatten one rendered line's spans into the plain string it displays.
pub fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Flatten a block of rendered lines, one line per `\n`.
///
/// Joining rather than concatenating means an assertion like
/// `contains("FPS  60.0")` cannot accidentally match across a line boundary.
pub fn lines_text(lines: &[Line<'_>]) -> String {
    lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
}

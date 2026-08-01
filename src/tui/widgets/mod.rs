//! Reusable pieces the views are assembled from.
//!
//! Nothing here owns state. Every function takes the values it draws plus a
//! theme and the Unicode flag, which keeps the widgets testable as pure
//! text-and-style producers.

pub mod chrome;
pub mod header;
pub mod help;
pub mod meter;
pub mod statusbar;
pub mod themes;

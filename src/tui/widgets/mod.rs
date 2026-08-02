//! Reusable pieces the views are assembled from.
//!
//! Nothing here owns state. Every function takes the values it draws plus a
//! theme and the Unicode flag, which keeps the widgets testable as pure
//! text-and-style producers.

pub mod banner;
pub mod chrome;
pub mod cmdline;
pub mod header;
pub mod help;
pub mod meter;
pub mod palette;
pub mod scrollbar;
pub mod statusbar;
pub mod themes;
pub mod whichkey;

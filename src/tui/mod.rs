//! The terminal UI.
//!
//! Split so that only [`run`] touches the terminal: [`app`] is a pure state
//! machine, [`keys`] a pure keymap, and everything under [`views`] and
//! [`widgets`] draws from a borrowed [`crate::metrics::Snapshot`] without
//! mutating anything. That means the interaction model and the layout maths
//! are testable without a TTY.

pub mod app;
pub mod command;
pub mod keys;
pub mod mouse;
pub mod run;
#[cfg(test)]
pub mod testing;
pub mod theme;
pub mod ui;
pub mod view;
pub mod views;
pub mod widgets;

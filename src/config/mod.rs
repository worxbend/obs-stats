//! Configuration: YAML on disk, environment for secrets, CLI flags on top.
//!
//! Precedence, lowest to highest: built-in defaults → config file → CLI flags.
//! Passwords are never part of that chain — they are read from an environment
//! variable named by the config, so a config file is always safe to commit.

pub mod loader;
pub mod model;
pub mod paths;

pub use loader::{load, resolve_password, write_default};
pub use model::{Config, ConnectionConfig, UiConfig};
pub use paths::{config_path, default_config_dir};

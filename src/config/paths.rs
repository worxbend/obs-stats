//! Where `obs-stats` looks for its files.
//!
//! `$OBS_STATS_CONFIG` wins outright — it names a file, not a directory — so
//! tests and one-off runs never touch the user's real config.

use std::path::PathBuf;

/// Environment variable that overrides the config file location entirely.
pub const CONFIG_PATH_ENV: &str = "OBS_STATS_CONFIG";

/// The XDG-style config directory, e.g. `~/.config/obs-stats`.
///
/// Falls back to `./.obs-stats` when the platform has no home directory,
/// which keeps the app usable inside minimal containers.
pub fn default_config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "obs-stats")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".obs-stats"))
}

/// The config file path, honouring [`CONFIG_PATH_ENV`].
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os(CONFIG_PATH_ENV)
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    default_config_dir().join("config.yml")
}

/// Where the debug log goes when `--log-file` is passed without a path.
pub fn default_log_path() -> PathBuf {
    default_config_dir().join("obs-stats.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_the_app_name() {
        let dir = default_config_dir();
        assert!(
            dir.ends_with("obs-stats") || dir.ends_with(".obs-stats"),
            "unexpected config dir: {}",
            dir.display()
        );
    }

    #[test]
    fn default_config_file_is_yaml_under_the_config_dir() {
        // Guard against an inherited override from the caller's environment.
        if std::env::var_os(CONFIG_PATH_ENV).is_some() {
            return;
        }
        let path = config_path();
        assert_eq!(path.file_name().unwrap(), "config.yml");
        assert_eq!(path.parent().unwrap(), default_config_dir());
    }

    #[test]
    fn log_path_sits_beside_the_config() {
        assert_eq!(
            default_log_path().parent(),
            Some(default_config_dir().as_path())
        );
    }
}

//! Reading, writing, and secret resolution for the config file.

use std::path::Path;

use crate::config::model::{Config, MAX_PASSWORD_LENGTH};
use crate::error::{Error, Redacted, Result};

/// The commented starter config written by `obs-stats init`.
pub const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../../config.example.yml");

/// Load and validate the config at `path`.
///
/// A missing file is not an error: `obs-stats` is expected to work on a fresh
/// machine with no setup, so defaults are returned instead.
pub fn load(path: &Path) -> Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            config.validate()?;
            return Ok(config);
        }
        Err(error) => {
            return Err(Error::ConfigRead(format!("{}: {error}", path.display())));
        }
    };

    parse(&text).map_err(|error| match error {
        // Prefix parse failures with the path; a bare serde message like
        // "unknown field `thmee`" is hard to act on when several configs exist.
        Error::ConfigInvalid(message) => {
            Error::ConfigInvalid(format!("{}: {message}", path.display()))
        }
        other => other,
    })
}

/// Parse and validate YAML config text.
pub fn parse(text: &str) -> Result<Config> {
    // An empty file means "all defaults"; serde_yaml would otherwise reject it.
    let config: Config = if text.trim().is_empty() {
        Config::default()
    } else {
        serde_yaml::from_str(text).map_err(|error| Error::ConfigInvalid(error.to_string()))?
    };
    config.validate()?;
    Ok(config)
}

/// Write the starter config to `path`, creating parent directories.
///
/// Refuses to clobber an existing file — losing a hand-tuned config to a
/// mistyped `init` would be worse than an error.
pub fn write_default(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::ConfigWrite(format!(
            "{} already exists; delete it first to regenerate",
            path.display()
        )));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::ConfigWrite(format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, DEFAULT_CONFIG_TEMPLATE)
        .map_err(|error| Error::ConfigWrite(format!("{}: {error}", path.display())))
}

/// Read the OBS password from the environment variable named by the config.
///
/// Returns `Ok(None)` when the variable is unset or empty, which is the normal
/// case for an OBS server with authentication turned off.
pub fn resolve_password(env_name: &str) -> Result<Option<Redacted>> {
    if env_name.is_empty() {
        return Ok(None);
    }

    let value = match std::env::var_os(env_name) {
        Some(value) => value,
        None => return Ok(None),
    };

    let value = value
        .into_string()
        .map_err(|_| Error::ConfigInvalid(format!("${env_name} is not valid UTF-8")))?;

    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_PASSWORD_LENGTH {
        return Err(Error::ConfigInvalid(format!(
            "${env_name} exceeds the maximum password length of {MAX_PASSWORD_LENGTH} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        // Usually a trailing newline from `export PW=$(cat file)`; failing
        // loudly beats an authentication error the user cannot explain.
        return Err(Error::ConfigInvalid(format!(
            "${env_name} contains control characters (a trailing newline?)"
        )));
    }

    Ok(Some(Redacted::new(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serializes the process-wide environment across tests in this module.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_env<R>(name: &str, value: Option<&str>, body: impl FnOnce() -> R) -> R {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(name);
        match value {
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
        let result = body();
        match previous {
            Some(previous) => unsafe { std::env::set_var(name, previous) },
            None => unsafe { std::env::remove_var(name) },
        }
        result
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("absent.yml")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn empty_file_yields_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
        assert_eq!(parse("   \n\n").unwrap(), Config::default());
    }

    #[test]
    fn shipped_example_config_is_valid() {
        // Guards against the documented example drifting from the schema.
        let example =
            parse(DEFAULT_CONFIG_TEMPLATE).expect("config.example.yml must parse and validate");
        assert_eq!(
            example,
            Config::default(),
            "starter config drifted from shipped defaults"
        );
    }

    #[test]
    fn parse_applies_validation() {
        let error = parse("ui:\n  poll_interval_ms: 5\n").unwrap_err();
        assert!(error.to_string().contains("poll_interval_ms"));
    }

    #[test]
    fn load_prefixes_parse_errors_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "ui:\n  theme: nonexistent\n").unwrap();
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("config.yml"), "{error}");
        assert!(error.contains("not a known theme"), "{error}");
    }

    #[test]
    fn load_reads_a_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "ui:\n  theme: nord\n  poll_interval_ms: 500\n").unwrap();
        let config = load(&path).unwrap();
        assert_eq!(config.ui.theme, "nord");
        assert_eq!(config.ui.poll_interval_ms, 500);
    }

    #[test]
    fn write_default_creates_parents_and_a_loadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/config.yml");
        write_default(&path).unwrap();
        assert!(path.exists());
        load(&path).expect("written config must load");
    }

    #[test]
    fn write_default_refuses_to_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        write_default(&path).unwrap();
        let error = write_default(&path).unwrap_err().to_string();
        assert!(error.contains("already exists"), "{error}");
    }

    #[test]
    fn password_is_read_from_the_named_variable() {
        with_env("OBS_STATS_TEST_PW", Some("hunter2"), || {
            let password = resolve_password("OBS_STATS_TEST_PW").unwrap().unwrap();
            assert_eq!(password.expose(), "hunter2");
            // And it must not print itself.
            assert_eq!(format!("{password:?}"), "<redacted>");
        });
    }

    #[test]
    fn unset_or_empty_variable_means_no_password() {
        with_env("OBS_STATS_TEST_PW", None, || {
            assert!(resolve_password("OBS_STATS_TEST_PW").unwrap().is_none());
        });
        with_env("OBS_STATS_TEST_PW", Some(""), || {
            assert!(resolve_password("OBS_STATS_TEST_PW").unwrap().is_none());
        });
        assert!(resolve_password("").unwrap().is_none());
    }

    #[test]
    fn a_trailing_newline_in_the_password_is_reported_clearly() {
        with_env("OBS_STATS_TEST_PW", Some("hunter2\n"), || {
            let error = resolve_password("OBS_STATS_TEST_PW")
                .unwrap_err()
                .to_string();
            assert!(error.contains("control characters"), "{error}");
            // The message must not echo the secret itself.
            assert!(!error.contains("hunter2"), "{error}");
        });
    }

    #[test]
    fn an_oversized_password_is_rejected() {
        let long = "a".repeat(MAX_PASSWORD_LENGTH + 1);
        with_env("OBS_STATS_TEST_PW", Some(&long), || {
            let error = resolve_password("OBS_STATS_TEST_PW")
                .unwrap_err()
                .to_string();
            assert!(error.contains("maximum password length"), "{error}");
        });
    }
}

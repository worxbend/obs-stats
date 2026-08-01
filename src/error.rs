//! Error type shared across the whole binary.
//!
//! Every variant is safe to print: nothing here ever carries a password. The
//! OBS client deliberately maps auth failures to [`Error::AuthFailed`] rather
//! than echoing the server comment, which can quote request payloads.

use std::fmt;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration is invalid: {0}")]
    ConfigInvalid(String),

    #[error("could not read configuration: {0}")]
    ConfigRead(String),

    #[error("could not write configuration: {0}")]
    ConfigWrite(String),

    #[error("could not connect to OBS at {url}: {reason}")]
    ConnectionFailed { url: String, reason: String },

    #[error("OBS rejected the connection: authentication failed")]
    AuthFailed,

    #[error("OBS WebSocket requires a password but none was configured")]
    AuthRequired,

    #[error("the OBS connection closed")]
    Disconnected,

    #[error("OBS did not answer {request} within {timeout_ms}ms")]
    RequestTimeout { request: String, timeout_ms: u64 },

    #[error("OBS request {request} failed (code {code}){}", detail(.comment))]
    RequestFailed {
        request: String,
        code: u32,
        comment: Option<String>,
    },

    #[error("unexpected message from OBS: {0}")]
    Protocol(String),

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

fn detail(comment: &Option<String>) -> String {
    match comment {
        Some(comment) if !comment.is_empty() => format!(": {comment}"),
        _ => String::new(),
    }
}

impl Error {
    /// Process exit code for this failure, so shell callers can branch on it.
    ///
    /// The mapping is part of the public contract and documented in the README;
    /// keep it stable across releases.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::ConfigInvalid(_) | Error::ConfigRead(_) | Error::ConfigWrite(_) => 2,
            Error::ConnectionFailed { .. } | Error::Disconnected => 3,
            Error::AuthFailed | Error::AuthRequired => 4,
            Error::RequestTimeout { .. } | Error::RequestFailed { .. } | Error::Protocol(_) => 5,
            Error::Terminal(_) | Error::Io(_) => 1,
        }
    }

    /// Whether a reconnect loop should keep retrying after this error.
    ///
    /// Bad credentials and bad config will fail identically on every retry, so
    /// they terminate the loop instead of spinning forever.
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::ConnectionFailed { .. }
            | Error::Disconnected
            | Error::RequestTimeout { .. }
            | Error::Io(_) => true,
            Error::AuthFailed
            | Error::AuthRequired
            | Error::ConfigInvalid(_)
            | Error::ConfigRead(_)
            | Error::ConfigWrite(_)
            | Error::RequestFailed { .. }
            | Error::Protocol(_)
            | Error::Terminal(_) => false,
        }
    }

    /// A short hint shown under the error in the TUI's disconnected banner.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Error::ConnectionFailed { .. } => Some(
                "Is OBS running with Tools → WebSocket Server Settings → Enable WebSocket server?",
            ),
            Error::AuthFailed => Some(
                "Check the password in OBS → Tools → WebSocket Server Settings → Show Connect Info.",
            ),
            Error::AuthRequired => Some(
                "Set OBS_WEBSOCKET_PASSWORD, or pass --password-env / connection.password_env.",
            ),
            _ => None,
        }
    }
}

/// Newtype whose `Debug`/`Display` never reveal the wrapped value.
///
/// Used for passwords so an accidental `{:?}` on a config struct — or a panic
/// backtrace that formats one — cannot leak the secret into logs.
#[derive(Clone, PartialEq, Eq)]
pub struct Redacted(pub String);

impl Redacted {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Display for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_never_prints_the_secret() {
        let secret = Redacted("hunter2".to_string());
        assert_eq!(format!("{secret:?}"), "<redacted>");
        assert_eq!(format!("{secret}"), "<redacted>");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn auth_errors_are_not_retryable() {
        assert!(!Error::AuthFailed.is_retryable());
        assert!(!Error::AuthRequired.is_retryable());
        assert!(Error::Disconnected.is_retryable());
        assert!(
            Error::ConnectionFailed {
                url: "ws://127.0.0.1:4455".into(),
                reason: "refused".into()
            }
            .is_retryable()
        );
    }

    #[test]
    fn exit_codes_are_grouped_by_failure_class() {
        assert_eq!(Error::ConfigInvalid("x".into()).exit_code(), 2);
        assert_eq!(Error::Disconnected.exit_code(), 3);
        assert_eq!(Error::AuthFailed.exit_code(), 4);
        assert_eq!(Error::Protocol("x".into()).exit_code(), 5);
    }

    #[test]
    fn request_failed_appends_comment_when_present() {
        let with = Error::RequestFailed {
            request: "GetStats".into(),
            code: 604,
            comment: Some("no active output".into()),
        };
        assert!(with.to_string().ends_with(": no active output"));

        let without = Error::RequestFailed {
            request: "GetStats".into(),
            code: 604,
            comment: None,
        };
        assert!(without.to_string().ends_with("(code 604)"));
    }
}

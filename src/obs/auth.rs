//! obs-websocket 5.x authentication.
//!
//! The scheme is a salted challenge-response:
//!
//! ```text
//! secret         = base64(sha256(password + salt))
//! authentication = base64(sha256(secret + challenge))
//! ```
//!
//! `salt` and `challenge` both arrive in the server's `Hello` frame, so the
//! password itself never crosses the wire.

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// Compute the `authentication` string for an `Identify` frame.
pub fn compute_authentication(password: &str, salt: &str, challenge: &str) -> String {
    let secret = {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hasher.update(salt.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
    };

    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(challenge.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_obs_websocket_spec_vector() {
        // Published example from the obs-websocket 5.x protocol document.
        let result = compute_authentication(
            "supersecretpassword",
            "PZVbYpvAnZut2SS3k3tnTQ==",
            "lfYW3AhFLp2YcILmwSQ9rSFRIiEQgxuEk5hSyQ3XGaQ=",
        );
        assert_eq!(result, "KyqYIxIYmV+kMWMia3ahAvmhvF16ReqnQK6KLN9onU4=");
    }

    #[test]
    fn is_deterministic() {
        let a = compute_authentication("pass", "salt", "challenge");
        let b = compute_authentication("pass", "salt", "challenge");
        assert_eq!(a, b);
    }

    #[test]
    fn every_input_affects_the_digest() {
        let base = compute_authentication("pass", "salt", "challenge");
        assert_ne!(base, compute_authentication("other", "salt", "challenge"));
        assert_ne!(base, compute_authentication("pass", "other", "challenge"));
        assert_ne!(base, compute_authentication("pass", "salt", "other"));
    }

    #[test]
    fn output_is_a_base64_sha256_digest() {
        let result = compute_authentication("pass", "salt", "challenge");
        // 32 raw bytes → 44 base64 characters including one '=' of padding.
        assert_eq!(result.len(), 44);
        assert!(result.ends_with('='));
    }
}

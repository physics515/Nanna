//! Which Ollama server an address names.
//!
//! The Ollama bearer token is bound to the server it was saved for, so
//! "is this the same server?" decides where a credential goes. It lives here,
//! once, because both the daemon (which sends the token) and the GUI (which
//! saves it) link this crate and nothing lower.

use reqwest::Url;

/// An Ollama base URL as it is saved: trimmed, without trailing slashes —
/// every call appends `/api/...` itself.
#[must_use]
pub fn normalize_ollama_host(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Whether two Ollama base URLs name the same server.
///
/// Compared as URLs, not strings: the scheme; the host case-insensitively,
/// with `localhost`, the IPv4 loopback block and `[::1]` all read as this
/// machine; the port, with a scheme's default the same written or not
/// (`https://host` is `https://host:443`); and the path without trailing
/// slashes, case-sensitively as paths are. User info, query and fragment do
/// not name a different server and are ignored. An address that does not
/// parse as a URL with a host matches nothing — the token stays home.
#[must_use]
pub fn same_ollama_server(a: &str, b: &str) -> bool {
    match (ServerIdentity::parse(a), ServerIdentity::parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// An address fit for a log line: user info (which can hold a password),
/// query and fragment dropped.
#[must_use]
pub fn redacted_ollama_host(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url.trim()) else {
        return "(an address that is not a URL)".to_string();
    };
    // Both fail only for URLs that cannot carry user info, which then have none.
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    normalize_ollama_host(parsed.as_str())
}

/// Say that the stored token is withheld from the configured server — once
/// per pair of addresses, never the token.
///
/// Secrets are hydrated on every config load, and the daemon reloads after
/// each settings save, so an unchanged mismatch would otherwise repeat the
/// same sentence on every save.
pub(crate) fn report_withheld_token(bound: &str, configured: &str) {
    static LAST_REPORTED: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
    let pair = (redacted_ollama_host(bound), redacted_ollama_host(configured));
    let mut last = LAST_REPORTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if last.as_ref() == Some(&pair) {
        return;
    }
    tracing::warn!(
        "The saved Ollama token belongs to {}, so it is not sent to the configured server {}. \
         Save a token for this server in Settings → Models if it needs one.",
        pair.0,
        pair.1
    );
    *last = Some(pair);
}

/// The parts of a URL that decide which server it reaches.
#[derive(Debug, PartialEq, Eq)]
struct ServerIdentity {
    scheme: String,
    host: HostIdentity,
    port: Option<u16>,
    path: String,
}

#[derive(Debug, PartialEq, Eq)]
enum HostIdentity {
    /// Every spelling of this machine — the clients pin `localhost` to
    /// `127.0.0.1`, and a server on `[::1]` is on the same machine.
    Loopback,
    Named(String),
}

impl ServerIdentity {
    fn parse(url: &str) -> Option<Self> {
        let parsed = Url::parse(url.trim()).ok()?;
        let host = parsed.host_str()?;
        let host = if crate::bind::is_loopback_host(host) {
            HostIdentity::Loopback
        } else {
            HostIdentity::Named(host.to_ascii_lowercase())
        };
        Some(Self {
            scheme: parsed.scheme().to_string(),
            host,
            port: parsed.port_or_known_default(),
            path: parsed.path().trim_end_matches('/').to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_ollama_host, redacted_ollama_host, same_ollama_server};

    #[test]
    fn spellings_of_one_server_match() {
        for (a, b) in [
            ("https://Host/ollama/", "https://host/ollama"),
            ("http://localhost:11434", " http://127.0.0.1:11434/ "),
            ("http://[::1]:11434", "http://localhost:11434"),
            ("http://127.0.0.2:11434", "http://[::1]:11434"),
            ("https://host:443/ollama", "https://host/ollama"),
            ("http://host:80", "http://host/"),
            ("https://user:pw@host/ollama", "https://host/ollama"),
        ] {
            assert!(same_ollama_server(a, b), "{a} and {b} are one server");
            assert!(same_ollama_server(b, a), "and the other way round");
        }
    }

    #[test]
    fn different_servers_do_not_match() {
        for (a, b) in [
            ("https://host/ollama", "https://host/api"),
            ("https://evil.example", "https://host/ollama"),
            ("http://host/ollama", "https://host/ollama"),
            ("https://host:8443/ollama", "https://host/ollama"),
            ("http://localhost:11434", "http://localhost:11435"),
            // Paths are case-sensitive: a proxy may route them apart.
            ("https://host/Ollama", "https://host/ollama"),
            // A loopback name is not a LAN host.
            ("http://localhost:11434", "http://gpu-box:11434"),
        ] {
            assert!(!same_ollama_server(a, b), "{a} and {b} are different servers");
        }
    }

    #[test]
    fn an_address_that_is_not_a_url_matches_nothing() {
        // `localhost:11434` parses as scheme `localhost` with no host at all.
        assert!(!same_ollama_server("localhost:11434", "localhost:11434"));
        assert!(!same_ollama_server("", ""));
        assert!(!same_ollama_server("not a url", "http://localhost:11434"));
    }

    #[test]
    fn a_saved_address_is_trimmed_of_space_and_trailing_slashes() {
        assert_eq!(normalize_ollama_host("  https://host/ollama// "), "https://host/ollama");
    }

    #[test]
    fn a_logged_address_carries_no_password() {
        assert_eq!(
            redacted_ollama_host("https://user:hunter2@host/ollama/?k=v#f"),
            "https://host/ollama"
        );
    }
}

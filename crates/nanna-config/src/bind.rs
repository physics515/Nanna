//! Network bind-address policy.
//!
//! Nanna's HTTP surfaces (the webhook receiver and the legacy `nanna server`)
//! used to default to `0.0.0.0`, which publishes them to every interface — a
//! LAN-visible, unauthenticated control surface on a personal machine, and a
//! footgun the moment the box sits on a café or hotel network.
//!
//! The default is now **loopback**, and reaching the outside world is an
//! explicit act: set `host` yourself. Servers log a warning when that happens,
//! so a public bind is always a deliberate, visible choice rather than an
//! inherited default. Users who front the webhook with a tunnel (cloudflared,
//! ngrok, a reverse proxy) are unaffected — those connect *to* loopback.

/// The default bind address for locally-scoped HTTP services.
pub const LOOPBACK_HOST: &str = "127.0.0.1";

/// Does `host` keep a listener on this machine only?
///
/// Recognises the IPv4 loopback block (`127.0.0.0/8` — the whole `/8`, since
/// `127.0.0.2` is just as local as `127.0.0.1`), the IPv6 loopback `::1` in its
/// bare and bracketed spellings, and `localhost`. Anything else — including the
/// wildcards `0.0.0.0` and `::` — is treated as public, which is the safe way to
/// be wrong: an unparseable or unfamiliar host is reported as public and merely
/// earns a warning, never a silent assumption of safety.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);

    if unbracketed.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // A parse failure means it is neither an IP literal nor `localhost` — a
    // hostname we cannot resolve without DNS. Fail safe: report it as public.
    unbracketed
        .parse::<std::net::IpAddr>()
        .is_ok_and(|addr| addr.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::{LOOPBACK_HOST, is_loopback_host};

    #[test]
    fn the_default_host_is_itself_loopback() {
        // The constant every server defaults to must satisfy the predicate that
        // decides whether to warn — otherwise a stock install warns about itself.
        assert!(is_loopback_host(LOOPBACK_HOST));
    }

    #[test]
    fn recognises_loopback_spellings() {
        for host in [
            "127.0.0.1",
            "127.0.0.2", // the whole 127/8 is loopback
            "127.255.255.254",
            "::1",
            "[::1]", // bracketed IPv6, as it appears in a host:port string
            "localhost",
            "LocalHost",   // case-insensitive
            " 127.0.0.1 ", // tolerate stray whitespace from a config file
        ] {
            assert!(is_loopback_host(host), "{host} must read as loopback");
        }
    }

    #[test]
    fn treats_wildcards_and_routable_addresses_as_public() {
        // Negative space — these are exactly the values that must trigger the
        // "you are publishing this" warning.
        for host in [
            "0.0.0.0", // IPv4 wildcard: every interface
            "::",      // IPv6 wildcard
            "[::]",
            "192.168.1.10",
            "10.0.0.5",
            "203.0.113.7",
            "example.com",
        ] {
            assert!(!is_loopback_host(host), "{host} must read as public");
        }
    }

    #[test]
    fn unparseable_hosts_fail_safe_to_public() {
        // We would rather warn about a host that turns out to be local than stay
        // silent about one that turns out to be routable.
        for host in ["", "   ", "not a host", "999.999.999.999", "[::1"] {
            assert!(!is_loopback_host(host), "{host:?} must fail safe to public");
        }
    }
}

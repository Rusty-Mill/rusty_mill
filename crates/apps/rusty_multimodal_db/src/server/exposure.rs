//! Refuse an exposed, unprotected listener at startup (`ADR-0094`,
//! `EXP-FR-001` through `EXP-FR-003`).
//!
//! `ServeOptions::default()` reproduces the original server exactly:
//! no tokens, no certificates, plaintext, every connection `ReadWrite`.
//! That is the right default for a library whose tests bind
//! `127.0.0.1:0` a thousand times, and the wrong one for a binary
//! someone starts on `0.0.0.0:7881` with nothing else set — which, until
//! this round, served every peer on the network read-write in the clear
//! and said so only in a banner. The README has always said "do not
//! expose beyond localhost unless auth *and* TLS are both configured";
//! this module makes the binaries enforce it.
//!
//! # The rule
//!
//! A bind address whose IP is loopback needs nothing: it is reachable
//! from this host only, the development posture every version allowed.
//! Any other address — an unspecified `0.0.0.0`/`[::]`, a LAN address,
//! or a hostname the check cannot classify — needs authentication
//! configured ([`ServeOptions::is_configured`]) *and* TLS
//! ([`ServeOptions::tls`]); missing either is an [`Exposure`] the binary
//! refuses to start on. `SERVER_ALLOW_INSECURE=1` overrides the refusal
//! for the operator who means it, with the same text as a warning.
//!
//! Library-level only in the sense that the rule is a pure function
//! here, so every binary applies the identical check and a test can pin
//! it; `serve`/`serve_tables` themselves are unchanged — a caller with
//! its own reasons still can bind whatever it likes.

use super::ServeOptions;
use std::fmt;
use std::net::SocketAddr;

/// The environment variable that turns a refusal into a warning.
pub const ALLOW_INSECURE_VAR: &str = "SERVER_ALLOW_INSECURE";

/// Why a non-loopback listener is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// No token and no certificate class is configured: every peer
    /// would start `ReadWrite`.
    NoAuth,
    /// Authentication is configured but the transport is plaintext:
    /// every token would cross the network in the clear.
    NoTls,
    /// `RVM-FR-004` (ADR-0111): the metrics HTTP listener has no
    /// authentication and no TLS at all, so a non-loopback bind would
    /// serve counters, table names, and traffic shape to the network.
    MetricsListener,
}

impl fmt::Display for Exposure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAuth => f.write_str(
                "no authentication is configured (set SERVER_AUTH_READ_WRITE_TOKEN and/or \
                 SERVER_AUTH_READ_ONLY_TOKEN, or client-certificate classes), so every peer \
                 on the network would be admitted read-write",
            ),
            Self::NoTls => f.write_str(
                "TLS is not configured (set SERVER_TLS_CERT_CHAIN_PATH and \
                 SERVER_TLS_PRIVATE_KEY_PATH), so every token would cross the network in \
                 the clear",
            ),
            Self::MetricsListener => f.write_str(
                "the metrics HTTP listener (SERVER_METRICS_HTTP_ADDR) has no authentication \
                 and no TLS, so every peer on the network could read the scrape",
            ),
        }
    }
}

impl std::error::Error for Exposure {}

/// `EXP-FR-001`: whether `addr` — the string a binary was asked to bind —
/// names a loopback IP. An address that does not parse as
/// `ip:port` (a hostname) is *not* loopback: the check cannot tell
/// what it resolves to, so it is treated as exposed.
pub fn bind_is_loopback(addr: &str) -> bool {
    addr.parse::<SocketAddr>()
        .map(|socket| socket.ip().is_loopback())
        .unwrap_or(false)
}

/// `EXP-FR-002`: `Ok` for a loopback bind, or for any bind with both
/// authentication and TLS configured; otherwise the first missing half.
pub fn check_exposure(addr: &str, options: &ServeOptions) -> Result<(), Exposure> {
    if bind_is_loopback(addr) {
        return Ok(());
    }
    if !options.is_configured() {
        return Err(Exposure::NoAuth);
    }
    if options.tls().is_none() {
        return Err(Exposure::NoTls);
    }
    Ok(())
}

/// `RVM-FR-004` (ADR-0111): the metrics HTTP listener has no protection
/// of its own, so only a loopback bind is ever `Ok`.
pub fn check_metrics_exposure(addr: &str) -> Result<(), Exposure> {
    if bind_is_loopback(addr) {
        Ok(())
    } else {
        Err(Exposure::MetricsListener)
    }
}

/// `EXP-FR-003`: whether the operator has set [`ALLOW_INSECURE_VAR`] to
/// exactly `1` — the one value that overrides a refusal.
pub fn allow_insecure_from_env() -> bool {
    std::env::var(ALLOW_INSECURE_VAR).is_ok_and(|v| v.trim() == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_addresses_are_recognised_and_everything_else_is_exposed() {
        assert!(bind_is_loopback("127.0.0.1:7881"));
        assert!(bind_is_loopback("127.5.6.7:1"));
        assert!(bind_is_loopback("[::1]:7881"));
        assert!(!bind_is_loopback("0.0.0.0:7881"));
        assert!(!bind_is_loopback("[::]:7881"));
        assert!(!bind_is_loopback("192.168.1.10:7881"));
        assert!(
            !bind_is_loopback("localhost:7881"),
            "a hostname is not classified"
        );
        assert!(!bind_is_loopback("not an address"));
    }

    #[test]
    fn a_loopback_bind_needs_nothing_and_an_exposed_one_needs_auth_then_tls() {
        let open = ServeOptions::new(None, None);
        assert_eq!(check_exposure("127.0.0.1:7881", &open), Ok(()));
        assert_eq!(check_exposure("0.0.0.0:7881", &open), Err(Exposure::NoAuth));

        let tokened = ServeOptions::new(None, Some("secret".into()));
        assert_eq!(check_exposure("[::1]:7881", &tokened), Ok(()));
        assert_eq!(
            check_exposure("0.0.0.0:7881", &tokened),
            Err(Exposure::NoTls)
        );
        assert_eq!(
            check_exposure("10.0.0.2:7881", &tokened),
            Err(Exposure::NoTls)
        );
    }

    #[test]
    fn the_messages_name_the_variables_an_operator_must_set() {
        let no_auth = Exposure::NoAuth.to_string();
        assert!(no_auth.contains("SERVER_AUTH_READ_WRITE_TOKEN"));
        let no_tls = Exposure::NoTls.to_string();
        assert!(no_tls.contains("SERVER_TLS_CERT_CHAIN_PATH"));
    }
}

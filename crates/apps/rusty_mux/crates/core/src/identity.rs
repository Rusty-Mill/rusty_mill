//! Who a client is, and how it reaches the server (C25).
//!
//! A [`Principal`] is what the server verified about a connection's peer,
//! never what the client claims. A personal server serves only its own
//! user (the socket's peer check refuses anyone else), so every client's
//! principal is that user; login mode (C26) will map other principals to
//! accounts. Over `rusty-mux --ssh` the peer is the SSH user's own process
//! on the server's host, so the SSH user is the principal there too.

use std::fmt;

use crate::{Error, MAX_NAME};

/// The identity the server verified for a connection: a stable `id` to
/// compare (`uid:1000` on Unix, `sid:S-1-5-…` on Windows) and the
/// account's `user` name for people to read, empty when it has none.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Principal {
    id: String,
    user: String,
}

impl Principal {
    /// Both fields at most [`MAX_NAME`] bytes, and `id` not empty: a
    /// principal travels on every client entry.
    pub fn new(id: impl Into<String>, user: impl Into<String>) -> Result<Self, Error> {
        let (id, user) = (id.into(), user.into());
        if id.is_empty() || id.len() > MAX_NAME || user.len() > MAX_NAME {
            return Err(Error::BadPrincipal);
        }
        Ok(Self { id, user })
    }

    /// A Unix account by number, with its name when it has one.
    pub fn unix(uid: u32, user: Option<String>) -> Result<Self, Error> {
        Self::new(format!("uid:{uid}"), user.unwrap_or_default())
    }

    /// The stable form to compare, such as `uid:1000`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The account name, empty when unknown.
    pub fn user(&self) -> &str {
        &self.user
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.user.as_str() {
            "" => f.write_str(&self.id),
            user => write!(f, "{user} ({})", self.id),
        }
    }
}

/// How a client's current connection reaches the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// The server's Unix socket: a local client, or one forwarded over SSH.
    Socket,
    /// A UDP association (`docs/projects/mux/UDP-TRANSPORT.md`).
    Datagram,
}

impl Transport {
    /// The name ctl and MCP print.
    pub fn name(self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::Datagram => "udp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_principal_is_bounded_and_never_anonymous() {
        assert_eq!(Principal::new("", "alice"), Err(Error::BadPrincipal));
        let long = "x".repeat(MAX_NAME + 1);
        assert_eq!(Principal::new(long.clone(), ""), Err(Error::BadPrincipal));
        assert_eq!(Principal::new("uid:1", long), Err(Error::BadPrincipal));
        let max = "x".repeat(MAX_NAME);
        assert!(Principal::new(max.clone(), max).is_ok());
    }

    #[test]
    fn a_unix_principal_is_its_uid_and_shows_its_name() {
        let named = Principal::unix(1000, Some("alice".into())).expect("valid");
        assert_eq!(named.id(), "uid:1000");
        assert_eq!(named.to_string(), "alice (uid:1000)");
        let unnamed = Principal::unix(7, None).expect("valid");
        assert_eq!(unnamed.to_string(), "uid:7");
    }
}

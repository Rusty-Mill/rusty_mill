//! The seam between the client and the wire: send a [`Message`], wait for the
//! next one. The child-process transport is [`crate::stdio`]; the Streamable
//! HTTP one is the next slice.

use rusty_mcp_proto::{Message, ProtocolVersion};
use std::io;
use std::time::Duration;

/// What [`Transport::recv`] produced.
#[derive(Debug)]
pub enum Recv {
    /// The next message.
    Message(Message),
    /// Nothing arrived within the wait.
    Timeout,
    /// The peer is gone.
    Closed,
}

/// Header changes for one outgoing HTTP request: what a proxy applies on
/// behalf of its caller to the call it forwards. Names are case-insensitive.
/// Transports without HTTP ignore it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderOverride {
    /// Headers to set, replacing any of the same name.
    pub set: Vec<(String, String)>,
    /// Header names to drop, standing ones included. Applied before `set`.
    pub remove: Vec<String>,
}

impl HeaderOverride {
    /// Whether there is nothing to change.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }

    /// Fold this into `headers`.
    pub fn apply(&self, headers: &mut Vec<(String, String)>) {
        let named = |h: &(String, String), n: &str| h.0.eq_ignore_ascii_case(n);
        for name in &self.remove {
            headers.retain(|h| !named(h, name));
        }
        for (name, value) in &self.set {
            headers.retain(|h| !named(h, name));
            headers.push((name.clone(), value.clone()));
        }
    }
}

/// A bidirectional stream of JSON-RPC messages.
pub trait Transport {
    /// Send one message.
    ///
    /// # Errors
    /// The underlying write failed.
    fn send(&mut self, message: &Message) -> io::Result<()>;

    /// [`Transport::send`] with `overrides` applied to the HTTP request that
    /// carries `message`. Transports that have no such request ignore them.
    ///
    /// # Errors
    /// As [`Transport::send`].
    fn send_with(&mut self, message: &Message, overrides: &HeaderOverride) -> io::Result<()> {
        let _ = overrides;
        self.send(message)
    }

    /// Wait up to `timeout` for the next message.
    ///
    /// # Errors
    /// The underlying read failed.
    fn recv(&mut self, timeout: Duration) -> io::Result<Recv>;

    /// The handshake settled on `version`. Transports that name the revision
    /// in every request (HTTP's `MCP-Protocol-Version`) start doing so;
    /// the others ignore it.
    fn set_protocol_version(&mut self, _version: &ProtocolVersion) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn set_replaces_a_header_of_any_case_and_remove_drops_it() {
        let mut headers = h(&[("Authorization", "Bearer a"), ("X-Trace", "t")]);
        HeaderOverride {
            set: h(&[("authorization", "Bearer b")]),
            remove: vec!["x-trace".to_owned()],
        }
        .apply(&mut headers);
        assert_eq!(headers, h(&[("authorization", "Bearer b")]));
    }

    #[test]
    fn remove_runs_before_set_so_a_name_in_both_ends_up_set() {
        let mut headers = h(&[("X-A", "1")]);
        HeaderOverride {
            set: h(&[("x-a", "2")]),
            remove: vec!["X-A".to_owned()],
        }
        .apply(&mut headers);
        assert_eq!(headers, h(&[("x-a", "2")]));
    }

    #[test]
    fn an_empty_override_changes_nothing() {
        let mut headers = h(&[("X-A", "1")]);
        let none = HeaderOverride::default();
        assert!(none.is_empty());
        none.apply(&mut headers);
        assert_eq!(headers, h(&[("X-A", "1")]));
    }
}

//! Forwarding a TLS connection without terminating it.
//!
//! The opposite of everything else in this crate: nothing is decrypted, no
//! certificate is presented, and the gateway never learns what is inside. It
//! reads one thing off the wire — the name in the ClientHello, which is
//! plaintext — picks a destination from it, and then copies bytes until one
//! side hangs up.
//!
//! # What routing can be, when nothing is read
//!
//! A path, a method, a header: all of them are inside the encryption. The only
//! thing a passthrough listener can route on is the server name the client
//! asked for, which is why a [`TcpRoute`] has hostnames and backends and
//! nothing else.
//!
//! A connection carrying no name at all — a client dialling by IP, or something
//! that is not TLS — matches only a route with no hostnames. That is the
//! catch-all, and a listener without one closes such a connection rather than
//! sending it to whichever route sorted first.
//!
//! # This is a different trust boundary
//!
//! A terminating listener can read, rewrite and refuse; every policy in this
//! gateway works because the bytes are in the clear at some point. None of that
//! is true here. `jwtAuth`, `extAuthz`, header modifiers, guards — none apply
//! to a connection nobody opens, and a route that expects them is a route that
//! does not have them. `Config::lint` says so rather than leaving it implied by
//! the absence of a `policies:` key.

use std::sync::Arc;

use agentgateway_config::TcpRoute;
use agentgateway_core::{Endpoints, HostnamePattern, Registry, RegistryError, resolve_backends};
use tokio::net::TcpStream;

/// A passthrough route that cannot be served.
#[derive(Debug, thiserror::Error)]
pub enum PassthroughError {
    /// A backend did not resolve.
    #[error(transparent)]
    Registry(#[from] RegistryError),

    /// The route's backends produced no address to dial.
    #[error(transparent)]
    Balance(#[from] agentgateway_core::BalanceError),
}

/// One compiled TCP route.
#[derive(Debug)]
struct Compiled {
    /// Most specific first; empty means the catch-all.
    hostnames: Vec<HostnamePattern>,
    endpoints: Endpoints,
}

/// The passthrough listeners on one port.
#[derive(Debug, Default)]
pub struct Passthrough {
    routes: Vec<Compiled>,
}

impl Passthrough {
    /// Compile the TCP routes of one port's listeners.
    ///
    /// Resolution happens here so a backend naming a service the inventory does
    /// not hold stops the gateway rather than closing every connection that
    /// arrives.
    pub fn new(
        routes: &[TcpRoute],
        registry: &Registry,
        at: &str,
    ) -> Result<Self, PassthroughError> {
        let mut compiled = Vec::new();

        for (r, route) in routes.iter().enumerate() {
            let at = format!("{at}.tcpRoutes[{r}]");
            let resolved = resolve_backends(&route.backends, registry, &at)?;
            let endpoints = Endpoints::new(
                resolved
                    .iter()
                    .map(|endpoint| (endpoint.authority.as_str(), endpoint.weight)),
                &at,
            )?;

            let mut hostnames: Vec<HostnamePattern> = route
                .hostnames
                .iter()
                .map(|hostname| HostnamePattern::parse(hostname))
                .collect();
            hostnames.sort_by_key(|pattern| std::cmp::Reverse(pattern.specificity()));

            compiled.push(Compiled {
                hostnames,
                endpoints,
            });
        }

        // Exact names before wildcards before the catch-all, so the route that
        // named a client's name outright wins over one that named its parent.
        compiled.sort_by_key(|route| {
            std::cmp::Reverse(
                route
                    .hostnames
                    .first()
                    .map_or(0, HostnamePattern::specificity),
            )
        });

        Ok(Passthrough { routes: compiled })
    }

    /// Whether this port forwards anything.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// The address a connection asking for `name` is forwarded to.
    ///
    /// `None` when nothing claims it. A route with no hostnames claims
    /// everything, including a connection that sent no name.
    fn destination(&self, name: Option<&str>) -> Option<String> {
        for route in &self.routes {
            let matched = match (route.hostnames.is_empty(), name) {
                // The catch-all takes anything, named or not.
                (true, _) => true,
                // A named route needs a name to compare against.
                (false, None) => false,
                (false, Some(name)) => route.hostnames.iter().any(|pattern| pattern.matches(name)),
            };
            if matched {
                return Some(route.endpoints.next().to_string());
            }
        }
        None
    }

    /// Forward one connection to completion.
    ///
    /// The client's bytes are peeked rather than read, so the ClientHello the
    /// name came from is still the first thing the upstream sees — it has to
    /// be, since the upstream is the one that will answer the handshake.
    pub async fn forward(self: Arc<Self>, client: TcpStream) {
        let name = crate::peek_server_name(&client).await;
        let Some(destination) = self.destination(name.as_deref()) else {
            tracing::debug!(
                server_name = name.as_deref().unwrap_or("<none>"),
                "closing a passthrough connection no route claims"
            );
            return;
        };

        let mut upstream = match TcpStream::connect(&destination).await {
            Ok(upstream) => upstream,
            Err(err) => {
                tracing::warn!(%destination, %err, "could not reach a passthrough upstream");
                return;
            }
        };

        let mut client = client;
        // Both directions at once, and both halves shut down when either side
        // finishes: a TLS session ends when one peer closes, and holding the
        // other open would leak a connection per handshake.
        match tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
            Ok((from_client, from_upstream)) => tracing::debug!(
                %destination,
                from_client,
                from_upstream,
                "a passthrough connection closed"
            ),
            Err(err) => {
                tracing::debug!(%destination, %err, "a passthrough connection failed")
            }
        }
    }
}

#[cfg(test)]
mod tests;

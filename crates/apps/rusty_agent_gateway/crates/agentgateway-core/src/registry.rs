//! Resolving a `service` backend to the addresses behind it.
//!
//! A route names a service; the inventory says which instances back it. This is
//! the join, done once at startup, so the request path sees the same
//! `(host:port, weight)` pairs a `host` backend produces and nothing downstream
//! has to know the difference.
//!
//! # Three ways to name one service
//!
//! `service.name` is matched against the full `namespace/hostname`, against the
//! hostname alone, and against the short name — in that order, most specific
//! first. Upstream's own examples use all three, and a file that names `api`
//! when two namespaces have one should be told it is ambiguous rather than
//! silently sent to whichever sorted first. See [`Registry::resolve`].
//!
//! # Weight is split, not repeated
//!
//! A route sending half its traffic to a service and half to a host means half,
//! however many instances the service has. Giving each instance the backend's
//! weight would make a three-instance service take three quarters instead.
//!
//! So the weights are scaled: every backend's weight is multiplied by the least
//! common multiple of the endpoint counts, then divided by its own count. With
//! one backend that is a no-op, and with several it makes the split exact in
//! integers — which is what [`crate::balance`]'s round-robin needs to hit a
//! ratio precisely rather than in expectation.
//!
//! # An empty service is a startup failure
//!
//! A service naming no reachable instance cannot serve a request, and a route
//! pointed at one would answer every call with an error. In a file-driven
//! inventory that is a typo rather than a cluster in flux — nothing is going to
//! come along later and fill it in — so it is refused at startup, where it is
//! one line of output rather than a pager.

use std::collections::BTreeMap;

use agentgateway_config::{
    Backend, BackendTarget, Health, NamedBackend, Service, ServiceRef, Workload,
};

/// A `service` backend that cannot be resolved.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// No service in the inventory answers to that name.
    #[error(
        "{at}: no service named `{name}` in the inventory; add it under `services:`, or use a \
         `host` backend for a literal address{}",
        known(known_names)
    )]
    Unknown {
        /// Where in the configuration it came from.
        at: String,
        /// The name that was asked for.
        name: String,
        /// What the inventory does hold.
        known_names: Vec<String>,
    },

    /// The name matched more than one service.
    #[error(
        "{at}: `{name}` matches {count} services in the inventory; name it as \
         `namespace/hostname` to say which"
    )]
    Ambiguous {
        /// Where in the configuration it came from.
        at: String,
        /// The name that was asked for.
        name: String,
        /// How many it matched.
        count: usize,
    },

    /// The service does not answer on that port.
    #[error(
        "{at}: service `{name}` does not answer on port {port}; it answers on {}",
        ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
    )]
    Port {
        /// Where in the configuration it came from.
        at: String,
        /// The service that was found.
        name: String,
        /// The port that was asked for.
        port: u16,
        /// The ports it does answer on.
        ports: Vec<u16>,
    },

    /// No backend in the list answers to that name.
    #[error("{at}: no backend named `{name}` in `backends:`{}", known(known_names))]
    UnknownBackend {
        /// Where in the configuration it came from.
        at: String,
        /// The name that was asked for.
        name: String,
        /// What the list does hold.
        known_names: Vec<String>,
    },

    /// The named backend has no address to dial.
    #[error(
        "{at}: backend `{name}` has no `host`, and only an address can be dialled here; an \
         `mcp` or `ai` backend is not one"
    )]
    BackendKind {
        /// Where in the configuration it came from.
        at: String,
        /// The backend that was found.
        name: String,
    },

    /// Nothing healthy backs the service.
    #[error(
        "{at}: service `{name}` has no healthy instance backing it, so this route could never \
         serve a request; a workload joins a service by naming it under `services:`"
    )]
    NoEndpoints {
        /// Where in the configuration it came from.
        at: String,
        /// The service that was found.
        name: String,
    },
}

/// Format the known names for an error, or nothing when there are none.
fn known(names: &[String]) -> String {
    match names.is_empty() {
        true => String::new(),
        false => format!(". The configuration holds: {}", names.join(", ")),
    }
}

/// One resolved address on a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `host:port`, ready to dial.
    pub authority: String,
    /// Share of the route's traffic, already scaled. See the module docs.
    pub weight: u32,
}

/// The service inventory, indexed for lookup.
#[derive(Debug, Default)]
pub struct Registry {
    /// `namespace/hostname` -> service.
    services: BTreeMap<String, Service>,
    /// `namespace/hostname` -> the instances that named it.
    endpoints: BTreeMap<String, Vec<Workload>>,
    /// Name -> the address a top-level `backends:` entry dials.
    backends: BTreeMap<String, NamedBackend>,
}

impl Registry {
    /// Index an inventory.
    ///
    /// Unhealthy instances are dropped here rather than at lookup, so
    /// everything downstream sees a set it can send traffic to.
    pub fn new(services: &[Service], workloads: &[Workload], backends: &[NamedBackend]) -> Self {
        let mut endpoints: BTreeMap<String, Vec<Workload>> = BTreeMap::new();
        for workload in workloads {
            if workload.status == Health::Unhealthy || workload.address().is_none() {
                continue;
            }
            for key in workload.services.keys() {
                endpoints
                    .entry(key.clone())
                    .or_default()
                    .push(workload.clone());
            }
        }

        Registry {
            services: services
                .iter()
                .map(|service| (service.key(), service.clone()))
                .collect(),
            endpoints,
            backends: backends
                .iter()
                .map(|backend| (backend.name.clone(), backend.clone()))
                .collect(),
        }
    }

    /// The address a named backend dials.
    ///
    /// Separate from [`Registry::resolve`] because a name in `backends:` is
    /// one address rather than a set: nothing joins it to an instance list, so
    /// there is nothing to load balance over.
    pub fn backend(&self, name: &str, at: &str) -> Result<String, RegistryError> {
        let backend = self
            .backends
            .get(name)
            .ok_or_else(|| RegistryError::UnknownBackend {
                at: at.to_string(),
                name: name.to_string(),
                known_names: self.backends.keys().cloned().collect(),
            })?;

        backend
            .host
            .clone()
            .ok_or_else(|| RegistryError::BackendKind {
                at: at.to_string(),
                name: name.to_string(),
            })
    }

    /// Whether anything can be resolved at all.
    pub fn is_empty(&self) -> bool {
        self.services.is_empty() && self.backends.is_empty()
    }

    /// The addresses behind one `service` backend.
    ///
    /// The weight returned per endpoint is the backend's own; splitting it
    /// across a route's backends is [`resolve_backends`]'s job, since that
    /// needs every backend's endpoint count to do it exactly.
    pub fn resolve(&self, reference: &ServiceRef, at: &str) -> Result<Vec<String>, RegistryError> {
        let key = self.lookup(&reference.name, at)?;
        let service = &self.services[&key];

        let target = service
            .target_port(reference.port)
            .ok_or_else(|| RegistryError::Port {
                at: at.to_string(),
                name: key.clone(),
                port: reference.port,
                ports: service.ports.keys().copied().collect(),
            })?;

        let addresses: Vec<String> = self
            .endpoints
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|workload| {
                let address = workload.address()?;
                // A workload's own map for this service wins over the
                // service's: it is the more specific statement, and it is how
                // one instance in a set differs from the rest.
                let port = workload
                    .services
                    .get(&key)
                    .and_then(|ports| ports.0.get(&reference.port).copied())
                    .unwrap_or(target);
                Some(format!("{address}:{port}"))
            })
            .collect();

        match addresses.is_empty() {
            true => Err(RegistryError::NoEndpoints {
                at: at.to_string(),
                name: key,
            }),
            false => Ok(addresses),
        }
    }

    /// Find the one service a name refers to.
    ///
    /// Most specific first: the full key, then the hostname, then the short
    /// name. A name matching several at the same specificity is ambiguous
    /// rather than resolved by sort order.
    fn lookup(&self, name: &str, at: &str) -> Result<String, RegistryError> {
        if self.services.contains_key(name) {
            return Ok(name.to_string());
        }

        for matches in [
            |service: &Service, name: &str| service.hostname == name,
            |service: &Service, name: &str| service.name == name,
        ] {
            let found: Vec<&String> = self
                .services
                .iter()
                .filter(|(_, service)| matches(service, name))
                .map(|(key, _)| key)
                .collect();
            match found.len() {
                0 => continue,
                1 => return Ok(found[0].clone()),
                count => {
                    return Err(RegistryError::Ambiguous {
                        at: at.to_string(),
                        name: name.to_string(),
                        count,
                    });
                }
            }
        }

        Err(RegistryError::Unknown {
            at: at.to_string(),
            name: name.to_string(),
            known_names: self.services.keys().cloned().collect(),
        })
    }
}

/// Every address a route's `host` and `service` backends resolve to.
///
/// The weights come back scaled so a route's split is exact whatever the
/// endpoint counts are; see the module docs for why repeating a backend's
/// weight per instance would not be.
pub fn resolve_backends(
    backends: &[Backend],
    registry: &Registry,
    at: &str,
) -> Result<Vec<Endpoint>, RegistryError> {
    // Resolve first, because the scale depends on how many endpoints each
    // backend turned out to have.
    let mut resolved: Vec<(Vec<String>, u32)> = Vec::new();
    for backend in backends {
        match &backend.target {
            BackendTarget::Host(host) => resolved.push((vec![host.clone()], backend.weight)),
            BackendTarget::Service(reference) => {
                resolved.push((registry.resolve(reference, at)?, backend.weight));
            }
            _ => {}
        }
    }

    // A backend contributing no endpoints cannot be scaled against and cannot
    // take traffic; `Endpoints` reports an empty route better than this can.
    let scale = resolved
        .iter()
        .map(|(addresses, _)| addresses.len() as u32)
        .filter(|count| *count > 0)
        .fold(1u32, lcm);

    Ok(resolved
        .into_iter()
        .flat_map(|(addresses, weight)| {
            let count = addresses.len() as u32;
            // `scale / count` is exact by construction: `scale` is a multiple
            // of every count. Saturating anyway, because a pathological
            // inventory should skew a ratio rather than panic a gateway.
            let each = weight
                .saturating_mul(scale.checked_div(count).unwrap_or(1))
                .max(u32::from(weight > 0));
            addresses.into_iter().map(move |authority| Endpoint {
                authority,
                // A drained backend stays drained: weight 0 in means 0 out,
                // and `max` above only lifts a non-zero weight that rounded
                // away.
                weight: match weight {
                    0 => 0,
                    _ => each,
                },
            })
        })
        .collect())
}

/// Least common multiple, saturating.
///
/// Bounded in practice by how many instances back a service, but a file can say
/// anything, and overflowing to zero here would silently drain a route.
fn lcm(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 {
        return a.max(b);
    }
    let product = u64::from(a) * u64::from(b);
    let divisor = u64::from(gcd(a, b));
    u32::try_from(product / divisor).unwrap_or(u32::MAX)
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

#[cfg(test)]
mod tests;

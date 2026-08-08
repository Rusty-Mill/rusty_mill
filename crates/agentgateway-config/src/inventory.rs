//! The service inventory a `service` backend resolves against.
//!
//! Upstream learns services and their endpoints from a control plane — xDS,
//! against an Istio-shaped inventory — and its local configuration file can
//! carry that same inventory written down by hand, as `services:` and
//! `workloads:` beside `binds:`. That is the half a file-driven gateway can
//! serve, and it is what this implements: no xDS client, but the same shapes,
//! so a `service` backend in an upstream file resolves here.
//!
//! # A service is a name; a workload is an address
//!
//! The two lists are joined rather than nested, exactly as the control plane
//! sends them. A [`Service`] declares a hostname and which ports it answers on;
//! a [`Workload`] declares an address and which services it backs. Nothing in a
//! service names its endpoints — a workload claims membership, not the other
//! way round — which is what lets a real control plane add and remove instances
//! without rewriting the service.
//!
//! The join key is `namespace/hostname`, and it is why [`Service::key`] exists
//! rather than the name being compared directly: two namespaces may each have
//! an `api`, and a workload in one must not resolve to the other's address.
//!
//! # Ports are mapped twice
//!
//! A service's `ports` maps the port a caller asks for onto the port a workload
//! listens on, and a workload's own entry for that service may map it again.
//! Both exist because a service can present `80` while its pods listen on
//! `8080`, and one pod in the set can differ from the rest. The workload's
//! answer wins where it has one, since it is the more specific statement.
//!
//! # What is parsed and not used
//!
//! Everything else the control plane sends — `vips`, `waypoint`, `locality`,
//! `subjectAltNames`, `loadBalancer` — is about a mesh this gateway is not part
//! of. It parses so an upstream file loads, and [`lint`] names it rather than
//! leaving an operator to assume a waypoint is being honoured.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

/// A service the control plane would otherwise supply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// Short name, as the control plane knows it.
    #[serde(default)]
    pub name: String,

    /// Namespace the service lives in.
    #[serde(default)]
    pub namespace: String,

    /// Fully qualified hostname, which is what a workload names.
    #[serde(default)]
    pub hostname: String,

    /// Port a caller asks for, mapped to the port a workload listens on.
    #[serde(
        default,
        deserialize_with = "port_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub ports: BTreeMap<u16, u16>,

    /// Everything else the control plane sends.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl Service {
    /// The `namespace/hostname` a workload names this service by.
    ///
    /// `hostname` falls back to `name`, because a hand-written entry usually
    /// has one or the other and upstream's own examples use both.
    pub fn key(&self) -> String {
        let host = match self.hostname.is_empty() {
            true => self.name.as_str(),
            false => self.hostname.as_str(),
        };
        format!("{}/{}", self.namespace, host)
    }

    /// The port a workload listens on for a port a caller asked for.
    ///
    /// `None` when the service does not answer on that port at all, which is a
    /// different thing from answering on it unmapped.
    pub fn target_port(&self, port: u16) -> Option<u16> {
        // An empty map is a service that maps nothing rather than one that
        // answers nothing: upstream's own local examples leave it off when the
        // service and target ports are the same.
        match self.ports.is_empty() {
            true => Some(port),
            false => self.ports.get(&port).copied(),
        }
    }
}

/// One instance backing zero or more services.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workload {
    /// Instance name, for logs.
    #[serde(default)]
    pub name: String,

    /// Namespace the instance runs in.
    #[serde(default)]
    pub namespace: String,

    /// Addresses it can be reached at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workload_ips: Vec<String>,

    /// A name to dial when it has no address of its own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hostname: String,

    /// Services it backs, by `namespace/hostname`, each with its own port map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, ServicePorts>,

    /// Whether it should receive traffic.
    #[serde(default)]
    pub status: Health,

    /// Everything else the control plane sends.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl Workload {
    /// The address to dial, if it has one.
    ///
    /// The first IP, or the hostname when there are none — a workload may be
    /// an external name rather than a pod.
    pub fn address(&self) -> Option<&str> {
        self.workload_ips
            .first()
            .map(String::as_str)
            .filter(|ip| !ip.is_empty())
            .or(match self.hostname.is_empty() {
                true => None,
                false => Some(self.hostname.as_str()),
            })
    }
}

/// A workload's port map for one service.
///
/// A newtype so the `{service port: target port}` map can be deserialized from
/// either integer or quoted keys; see [`port_map`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServicePorts(#[serde(deserialize_with = "port_map")] pub BTreeMap<u16, u16>);

/// Whether an endpoint should receive traffic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    /// Send it traffic. The default: an entry written down without a status is
    /// one somebody expects to be used.
    #[default]
    Healthy,
    /// Leave it out of the set.
    Unhealthy,
}

/// Deserialize a `{port: port}` map from integer or string keys.
///
/// YAML writes `8080: 80` and JSON cannot — its object keys are strings — so a
/// configuration converted from one to the other would otherwise stop parsing.
/// Accepting both costs nothing and removes a trap that only shows up after a
/// conversion.
fn port_map<'de, D>(deserializer: D) -> Result<BTreeMap<u16, u16>, D::Error>
where
    D: Deserializer<'de>,
{
    /// A key written either way.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PortKey {
        Number(u64),
        Text(String),
    }

    struct Ports;

    impl<'de> serde::de::Visitor<'de> for Ports {
        type Value = BTreeMap<u16, u16>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map of port numbers to port numbers")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            use serde::de::Error as _;

            let mut ports = BTreeMap::new();
            while let Some(key) = map.next_key::<PortKey>()? {
                let parsed = match &key {
                    PortKey::Number(number) => Some(*number),
                    PortKey::Text(text) => text.parse::<u64>().ok(),
                };
                let port = parsed
                    .and_then(|port| u16::try_from(port).ok())
                    .ok_or_else(|| match &key {
                        PortKey::Number(number) => {
                            A::Error::custom(format!("`{number}` is not a port number"))
                        }
                        PortKey::Text(text) => {
                            A::Error::custom(format!("`{text}` is not a port number"))
                        }
                    })?;
                ports.insert(port, map.next_value()?);
            }
            Ok(ports)
        }
    }

    deserializer.deserialize_map(Ports)
}

/// Report inventory fields that parse and do not act.
pub(crate) fn lint(services: &[Service], workloads: &[Workload], findings: &mut Vec<String>) {
    for (i, service) in services.iter().enumerate() {
        for key in service.rest.keys() {
            findings.push(format!(
                "services[{i}].{key}: parsed but not used by this build; it describes a mesh \
                 this gateway is not part of"
            ));
        }
    }
    for (i, workload) in workloads.iter().enumerate() {
        for key in workload.rest.keys() {
            findings.push(format!(
                "workloads[{i}].{key}: parsed but not used by this build; it describes a mesh \
                 this gateway is not part of"
            ));
        }
    }
}

#[cfg(test)]
mod tests;

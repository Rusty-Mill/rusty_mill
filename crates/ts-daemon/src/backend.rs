//! Adapter from the LocalAPI [`LocalBackend`] port to the engine handle.
//!
//! `ts-localapi` handles the HTTP/UDS wire; this bridges its three operations
//! to `EngineHandle`, so an unmodified `ts-cli` (or the Go `tailscale` CLI)
//! drives our daemon. Ports and adapters: the server knows nothing of the
//! engine, the engine nothing of HTTP.

use std::net::IpAddr;
use std::time::Duration;

use ts_engine::{EngineHandle, PingError};
use ts_localapi::LocalBackend;
use ts_types::{MaskedPrefs, PingResult, Prefs, Status};

/// LocalAPI ping timeout; the first ping to a peer includes the WireGuard
/// handshake, so allow generous time (matches `ts-daemon --ping`).
const PING_TIMEOUT: Duration = Duration::from_secs(10);

pub struct DaemonBackend {
    engine: EngineHandle,
    control_url: String,
    hostname: String,
}

impl DaemonBackend {
    pub fn new(engine: EngineHandle, control_url: String, hostname: String) -> Self {
        Self {
            engine,
            control_url,
            hostname,
        }
    }
}

impl LocalBackend for DaemonBackend {
    async fn status(&self) -> Status {
        self.engine.status().await.unwrap_or_else(|| Status {
            backend_state: "NoState".into(),
            ..Default::default()
        })
    }

    async fn edit_prefs(&self, masked: MaskedPrefs) -> Prefs {
        if let Some(want) = masked.want_running {
            self.engine.set_want_running(want).await;
        }
        let st = self.engine.status().await.unwrap_or_default();
        Prefs {
            control_url: self.control_url.clone(),
            hostname: self.hostname.clone(),
            want_running: st.backend_state == "Running",
            ..Default::default()
        }
    }

    async fn ping(&self, ip: IpAddr) -> PingResult {
        let IpAddr::V4(v4) = ip else {
            return PingResult {
                ip: ip.to_string(),
                err: "IPv6 ping not supported".into(),
                ..Default::default()
            };
        };

        match self.engine.ping(v4, PING_TIMEOUT).await {
            Ok(rtt) => {
                // Enrich the result with the peer's name and current path from
                // the status snapshot.
                let st = self.engine.status().await.unwrap_or_default();
                let peer = st.peer.values().find(|p| p.tailscale_ips.contains(&ip));
                let (node_name, endpoint, relay) = match peer {
                    Some(p) => (p.name().to_string(), p.cur_addr.clone(), p.relay.clone()),
                    None => (String::new(), String::new(), String::new()),
                };
                // Report the DERP region code only when the path is relayed
                // (no direct endpoint), so the CLI prints "via DERP(...)".
                let derp_region_code = if endpoint.is_empty() {
                    relay
                } else {
                    String::new()
                };
                PingResult {
                    ip: ip.to_string(),
                    node_ip: ip.to_string(),
                    node_name,
                    latency_seconds: rtt.as_secs_f64(),
                    endpoint,
                    derp_region_code,
                    ..Default::default()
                }
            }
            Err(e) => {
                let err = match e {
                    PingError::UnknownPeer(_) => format!("no peer with tailnet IP {ip}"),
                    other => other.to_string(),
                };
                PingResult {
                    ip: ip.to_string(),
                    node_ip: ip.to_string(),
                    err,
                    ..Default::default()
                }
            }
        }
    }
}

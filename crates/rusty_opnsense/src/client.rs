use std::time::Duration;

use rusty_request::{Client, TrustPolicy};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::ServiceAction;

/// Where to find an OPNsense firewall and how to authenticate to it.
///
/// OPNsense's API key/secret pair (sent as HTTP Basic auth, key as username)
/// is the only auth this crate speaks -- generated per user under **System
/// -> Access -> Users**, "API keys" section, and independent of that user's
/// own login password.
#[derive(Debug, Clone)]
pub struct OpnsenseConfig {
    /// The API base URL, e.g. `https://opnsense.lan`. No trailing slash
    /// needed -- one is stripped if present.
    pub base_url: String,
    /// The API key.
    pub key: String,
    /// The API secret, shown once when the key is generated.
    pub secret: String,
    /// Skip TLS certificate verification. OPNsense ships a self-signed
    /// certificate by default, which most homelabs never replace -- set this
    /// rather than reaching for `https://` without a trust story at all.
    /// Never set it for a host reachable outside a trusted network.
    pub insecure: bool,
    /// Per-request timeout. `None` uses `rusty_request`'s own default (30s).
    pub timeout: Option<Duration>,
}

/// An async client for one OPNsense firewall's REST API.
///
/// Unlike Proxmox's uniform `{"data": ...}` envelope, OPNsense's response
/// shape varies by endpoint (a plain object here, `{"rows": [...], ...}`
/// there), so every method hands back the parsed body as-is rather than
/// unwrapping a field that isn't consistently present. Cheap to clone -- it
/// shares the same underlying `rusty_request::Client` (connection pool
/// included).
#[derive(Debug, Clone)]
pub struct OpnsenseClient {
    http: Client,
    base_url: String,
    key: String,
    secret: String,
}

impl OpnsenseClient {
    /// Build a client. Does not connect -- the first real request is
    /// whatever method is called first.
    pub fn new(config: OpnsenseConfig) -> Self {
        let trust_policy = if config.insecure {
            TrustPolicy::DangerNoVerification
        } else {
            TrustPolicy::System
        };
        let mut builder = Client::builder().trust_policy(trust_policy);
        if let Some(timeout) = config.timeout {
            builder = builder.timeout(timeout);
        }
        Self {
            http: builder.build(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            key: config.key,
            secret: config.secret,
        }
    }

    /// `GET /core/system/status` -- firmware version, running kernel,
    /// pending updates, and per-service health.
    pub async fn system_status(&self) -> Result<Value> {
        self.get("/api/core/system/status").await
    }

    /// `GET /core/service/search` -- every service OPNsense's service
    /// supervisor knows about, with its running state.
    pub async fn list_services(&self) -> Result<Value> {
        self.get("/api/core/service/search").await
    }

    /// `POST /core/service/{action}/{name}` -- start, stop, or restart a
    /// named service (the short id from [`OpnsenseClient::list_services`],
    /// e.g. `unbound`, `dhcpd`, `sshd`).
    pub async fn service_control(&self, name: &str, action: ServiceAction) -> Result<Value> {
        self.post(&format!("/api/core/service/{action}/{name}"))
            .await
    }

    /// `GET /diagnostics/interface/getInterfaceNames` -- every network
    /// interface OPNsense knows about, keyed by device name.
    pub async fn list_interfaces(&self) -> Result<Value> {
        self.get("/api/diagnostics/interface/getInterfaceNames")
            .await
    }

    /// `GET /firewall/alias/export` -- every firewall alias currently
    /// configured, in the same JSON shape the "download" button on the
    /// Aliases page produces.
    pub async fn list_firewall_aliases(&self) -> Result<Value> {
        self.get("/api/firewall/alias/export").await
    }

    /// `GET /routes/gateway/status` -- every configured gateway, with its
    /// monitor status (`none`/`loss`/`down`) and ping latency.
    pub async fn list_gateways(&self) -> Result<Value> {
        self.get("/api/routes/gateway/status").await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(&format!("{}{path}", self.base_url))?
            .basic_auth(&self.key, &self.secret)?
            .send()
            .await?;
        Self::parse(response).await
    }

    async fn post(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .post(&format!("{}{path}", self.base_url))?
            .basic_auth(&self.key, &self.secret)?
            .send()
            .await?;
        Self::parse(response).await
    }

    async fn parse(response: rusty_request::Response) -> Result<Value> {
        let status = response.status();
        let text = response.text()?;
        if status.is_client_error() || status.is_server_error() {
            return Err(Error::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        Ok(serde_json::from_str(&text)?)
    }
}

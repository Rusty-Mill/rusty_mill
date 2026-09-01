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
///
/// Firewall rule writes (`create_firewall_rule`/`update_firewall_rule`/
/// `delete_firewall_rule`/`toggle_firewall_rule`) and VLAN writes
/// (`create_vlan`/`update_vlan`/`delete_vlan`) don't take effect on their
/// own -- OPNsense buffers each config area's changes until its own
/// `apply_*_changes` method is called, the same way the web UI's "Apply
/// changes" banner implies.
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

    /// `GET /firewall/filter/searchRule` -- every firewall rule currently
    /// configured (a `{"rows": [...], "rowCount": N, ...}` search envelope,
    /// same shape as [`OpnsenseClient::list_services`]).
    pub async fn list_firewall_rules(&self) -> Result<Value> {
        self.get("/api/firewall/filter/searchRule").await
    }

    /// `GET /firewall/filter/getRule/{uuid}` -- one rule's full field set.
    pub async fn get_firewall_rule(&self, uuid: &str) -> Result<Value> {
        self.get(&format!("/api/firewall/filter/getRule/{uuid}"))
            .await
    }

    /// `POST /firewall/filter/addRule` -- create a rule. `fields` is the
    /// field set OPNsense's own rule form submits (`action`, `interface`,
    /// `direction`, `protocol`, `source_net`, `destination_net`,
    /// `description`, ...) -- passed through as-is rather than modeled,
    /// since the valid field set depends on the rule's own
    /// `ipprotocol`/`protocol` rather than being one fixed schema. Doesn't
    /// take effect until [`OpnsenseClient::apply_firewall_changes`] is
    /// called.
    pub async fn create_firewall_rule(&self, fields: Value) -> Result<Value> {
        self.post_json(
            "/api/firewall/filter/addRule",
            &serde_json::json!({ "rule": fields }),
        )
        .await
    }

    /// `POST /firewall/filter/setRule/{uuid}` -- update a rule. Same
    /// passthrough field set as [`OpnsenseClient::create_firewall_rule`];
    /// OPNsense replaces the rule with exactly what's sent, so read
    /// [`OpnsenseClient::get_firewall_rule`] first and send back its full
    /// field set unless clearing the fields you omit is intended. Doesn't
    /// take effect until [`OpnsenseClient::apply_firewall_changes`] is
    /// called.
    pub async fn update_firewall_rule(&self, uuid: &str, fields: Value) -> Result<Value> {
        self.post_json(
            &format!("/api/firewall/filter/setRule/{uuid}"),
            &serde_json::json!({ "rule": fields }),
        )
        .await
    }

    /// `POST /firewall/filter/delRule/{uuid}` -- delete a rule. Doesn't take
    /// effect until [`OpnsenseClient::apply_firewall_changes`] is called.
    pub async fn delete_firewall_rule(&self, uuid: &str) -> Result<Value> {
        self.post(&format!("/api/firewall/filter/delRule/{uuid}"))
            .await
    }

    /// `POST /firewall/filter/toggleRule/{uuid}[/{0,1}]` -- flip a rule's
    /// enabled state, or set it explicitly when `enabled` is given. Doesn't
    /// take effect until [`OpnsenseClient::apply_firewall_changes`] is
    /// called.
    pub async fn toggle_firewall_rule(&self, uuid: &str, enabled: Option<bool>) -> Result<Value> {
        let path = match enabled {
            Some(true) => format!("/api/firewall/filter/toggleRule/{uuid}/1"),
            Some(false) => format!("/api/firewall/filter/toggleRule/{uuid}/0"),
            None => format!("/api/firewall/filter/toggleRule/{uuid}"),
        };
        self.post(&path).await
    }

    /// `POST /firewall/filter/apply` -- apply every pending rule change
    /// (reloads the live ruleset). OPNsense buffers create/update/delete/
    /// toggle above until this is called -- none of them take effect on
    /// their own.
    pub async fn apply_firewall_changes(&self) -> Result<Value> {
        self.post("/api/firewall/filter/apply").await
    }

    /// `POST /dhcpv4/leases/searchLease` -- every current DHCP lease (a
    /// `{"rows": [...], "rowCount": N, ...}` search envelope, same shape as
    /// [`OpnsenseClient::list_services`]).
    pub async fn list_dhcp_leases(&self) -> Result<Value> {
        self.post_json("/api/dhcpv4/leases/searchLease", &serde_json::json!({}))
            .await
    }

    /// `POST /interfaces/vlan_settings/searchItem` -- every configured VLAN
    /// interface (search envelope, same shape as
    /// [`OpnsenseClient::list_dhcp_leases`]).
    pub async fn list_vlans(&self) -> Result<Value> {
        self.post_json(
            "/api/interfaces/vlan_settings/searchItem",
            &serde_json::json!({}),
        )
        .await
    }

    /// `GET /interfaces/vlan_settings/getItem/{uuid}` -- one VLAN's full
    /// field set.
    pub async fn get_vlan(&self, uuid: &str) -> Result<Value> {
        self.get(&format!("/api/interfaces/vlan_settings/getItem/{uuid}"))
            .await
    }

    /// `POST /interfaces/vlan_settings/addItem` -- create a VLAN. `fields`
    /// is the same field set OPNsense's own VLAN form submits (`if` the
    /// parent interface, `tag`, `descr`, `pcp`) -- passed through as-is,
    /// same reasoning as [`OpnsenseClient::create_firewall_rule`]. Doesn't
    /// take effect until [`OpnsenseClient::apply_vlan_changes`] is called.
    pub async fn create_vlan(&self, fields: Value) -> Result<Value> {
        self.post_json(
            "/api/interfaces/vlan_settings/addItem",
            &serde_json::json!({ "vlan": fields }),
        )
        .await
    }

    /// `POST /interfaces/vlan_settings/setItem/{uuid}` -- update a VLAN.
    /// Same passthrough field set as [`OpnsenseClient::create_vlan`].
    /// Doesn't take effect until [`OpnsenseClient::apply_vlan_changes`] is
    /// called.
    pub async fn update_vlan(&self, uuid: &str, fields: Value) -> Result<Value> {
        self.post_json(
            &format!("/api/interfaces/vlan_settings/setItem/{uuid}"),
            &serde_json::json!({ "vlan": fields }),
        )
        .await
    }

    /// `POST /interfaces/vlan_settings/delItem/{uuid}` -- delete a VLAN.
    /// Doesn't take effect until [`OpnsenseClient::apply_vlan_changes`] is
    /// called.
    pub async fn delete_vlan(&self, uuid: &str) -> Result<Value> {
        self.post(&format!("/api/interfaces/vlan_settings/delItem/{uuid}"))
            .await
    }

    /// `POST /interfaces/vlan_settings/reconfigure` -- apply every pending
    /// VLAN change. OPNsense buffers create/update/delete above until this
    /// is called -- none of them take effect on their own.
    pub async fn apply_vlan_changes(&self) -> Result<Value> {
        self.post("/api/interfaces/vlan_settings/reconfigure").await
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

    async fn post_json(&self, path: &str, payload: &Value) -> Result<Value> {
        let response = self
            .http
            .post(&format!("{}{path}", self.base_url))?
            .basic_auth(&self.key, &self.secret)?
            .header("Content-Type", "application/json")?
            .body(serde_json::to_string(payload)?)
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

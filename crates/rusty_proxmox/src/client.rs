use std::time::Duration;

use rusty_request::{Client, TrustPolicy};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{GuestKind, PowerAction};

/// Where to find a Proxmox VE cluster/node and how to authenticate to it.
///
/// Proxmox's API token auth (`PVEAPIToken=<user>@<realm>!<token-id>=<secret>`)
/// is the only auth this crate speaks -- it's the form meant for
/// automation, unlike the ticket/CSRF-token pair the web UI uses.
#[derive(Debug, Clone)]
pub struct ProxmoxConfig {
    /// The API base URL, e.g. `https://pve.lan:8006`. No trailing slash
    /// needed -- one is stripped if present.
    pub base_url: String,
    /// `<user>@<realm>!<token-id>`, e.g. `automation@pve!homelab-mcp`.
    pub token_id: String,
    /// The token's secret, shown once when the token is created in
    /// Datacenter -> Permissions -> API Tokens.
    pub token_secret: String,
    /// Skip TLS certificate verification. Proxmox ships a self-signed
    /// certificate by default, which most homelabs never replace -- set this
    /// rather than reaching for `https://` without a trust story at all.
    /// Never set it for a host reachable outside a trusted network.
    pub insecure: bool,
    /// Per-request timeout. `None` uses `rusty_request`'s own default (30s).
    pub timeout: Option<Duration>,
}

/// An async client for one Proxmox VE cluster/node's REST API.
///
/// Every method returns the response's `data` field as-is (every Proxmox API
/// response is documented to wrap its payload that way), except
/// [`ProxmoxClient::guest_power`], which unwraps the task UPID string Proxmox
/// hands back for an asynchronous action. Cheap to clone -- it shares the
/// same underlying `rusty_request::Client` (connection pool included).
#[derive(Debug, Clone)]
pub struct ProxmoxClient {
    http: Client,
    base_url: String,
    auth_header: String,
}

impl ProxmoxClient {
    /// Build a client. Does not connect -- the first real request is
    /// whatever method is called first.
    pub fn new(config: ProxmoxConfig) -> Self {
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
            auth_header: format!("PVEAPIToken={}={}", config.token_id, config.token_secret),
        }
    }

    /// `GET /nodes` -- every node in the cluster, with its overall status
    /// (`online`/`offline`), CPU/memory usage, and uptime.
    pub async fn list_nodes(&self) -> Result<Value> {
        self.get("/api2/json/nodes").await
    }

    /// `GET /nodes/{node}/status` -- one node's detailed status (CPU, memory,
    /// swap, load average, kernel version, uptime).
    pub async fn node_status(&self, node: &str) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/status")).await
    }

    /// `GET /nodes/{node}/qemu` or `.../lxc` -- every guest of the given kind
    /// on that node, with `vmid`, `name`, and `status` (`running`/`stopped`).
    pub async fn list_guests(&self, node: &str, kind: GuestKind) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/{kind}")).await
    }

    /// `GET /nodes/{node}/{qemu,lxc}/{vmid}/status/current` -- one guest's
    /// live status: run state, uptime, CPU/memory usage, configured
    /// resources.
    pub async fn guest_status(&self, node: &str, kind: GuestKind, vmid: u32) -> Result<Value> {
        self.get(&format!(
            "/api2/json/nodes/{node}/{kind}/{vmid}/status/current"
        ))
        .await
    }

    /// `POST /nodes/{node}/{qemu,lxc}/{vmid}/status/{action}` -- start, stop,
    /// shut down, reboot, suspend, or resume a guest.
    ///
    /// Proxmox runs this asynchronously and returns a task ID (a `UPID:...`
    /// string) rather than waiting for the action to finish; poll
    /// [`ProxmoxClient::task_status`] with it to find out when it completes.
    pub async fn guest_power(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        action: PowerAction,
    ) -> Result<String> {
        let path = format!("/api2/json/nodes/{node}/{kind}/{vmid}/status/{action}");
        let data = self.post(&path).await?;
        data.as_str().map(str::to_string).ok_or_else(|| {
            Error::MissingData(format!("expected a UPID string in `data`, got {data}"))
        })
    }

    /// `GET /nodes/{node}/tasks/{upid}/status` -- whether an asynchronous
    /// task (the UPID string returned by `guest_power` and every other
    /// action Proxmox runs in the background) is still running, and if not,
    /// whether it succeeded (`status: "OK"` vs. an error message).
    pub async fn task_status(&self, node: &str, upid: &str) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/tasks/{upid}/status"))
            .await
    }

    /// `GET /nodes/{node}/tasks/{upid}/log` -- an asynchronous task's log
    /// output, most useful for finding out *why* a task in `task_status`
    /// failed.
    pub async fn task_log(&self, node: &str, upid: &str) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/tasks/{upid}/log"))
            .await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(&format!("{}{path}", self.base_url))?
            .header("Authorization", &self.auth_header)?
            .send()
            .await?;
        Self::unwrap_data(response).await
    }

    async fn post(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .post(&format!("{}{path}", self.base_url))?
            .header("Authorization", &self.auth_header)?
            .send()
            .await?;
        Self::unwrap_data(response).await
    }

    async fn unwrap_data(response: rusty_request::Response) -> Result<Value> {
        let status = response.status();
        let text = response.text()?;
        if status.is_client_error() || status.is_server_error() {
            return Err(Error::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        let mut body: Value = serde_json::from_str(&text)?;
        match body.as_object_mut().and_then(|obj| obj.remove("data")) {
            Some(data) => Ok(data),
            None => Err(Error::MissingData(text)),
        }
    }
}

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
/// response is documented to wrap its payload that way), except every
/// asynchronous action (`guest_power`, `create_guest`, `delete_guest`,
/// `clone_guest`, `create_snapshot`, `delete_snapshot`,
/// `rollback_snapshot`), which unwraps the task UPID string Proxmox hands
/// back instead of waiting for the action to finish -- poll it with
/// [`ProxmoxClient::task_status`]/[`ProxmoxClient::task_log`]. Cheap to
/// clone -- it shares the same underlying `rusty_request::Client`
/// (connection pool included).
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
        self.post(&path).await.and_then(Self::expect_upid)
    }

    /// `GET /nodes/{node}/{qemu,lxc}/{vmid}/config` -- one guest's current
    /// configuration: CPU, memory, disks, network interfaces, boot order,
    /// and everything else Proxmox stores per-guest.
    pub async fn guest_config(&self, node: &str, kind: GuestKind, vmid: u32) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/{kind}/{vmid}/config"))
            .await
    }

    /// `PUT /nodes/{node}/{qemu,lxc}/{vmid}/config` -- update a guest's
    /// configuration. `fields` is the same field set Proxmox's own config
    /// API takes (`cores`, `memory`, `net0`, `scsi0`, ...) -- passed through
    /// as-is, since the valid field set differs between QEMU and LXC and by
    /// what's already configured. Usually synchronous (`data` is `null`),
    /// but some fields (e.g. a disk resize) make Proxmox run the update as
    /// a background task instead, returning its UPID -- check the returned
    /// value's type rather than assuming either.
    pub async fn update_guest_config(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        fields: Value,
    ) -> Result<Value> {
        self.put_json(
            &format!("/api2/json/nodes/{node}/{kind}/{vmid}/config"),
            &fields,
        )
        .await
    }

    /// `POST /nodes/{node}/{qemu,lxc}` -- create a new guest. `fields` must
    /// include `vmid`; the rest is the same field set Proxmox's own
    /// create-guest API takes, and differs between QEMU (`ostype`,
    /// `scsi0`, `net0`, ...) and LXC (`ostemplate`, `rootfs`, ...) --
    /// passed through as-is. Runs asynchronously; returns the task UPID.
    pub async fn create_guest(&self, node: &str, kind: GuestKind, fields: Value) -> Result<String> {
        self.post_json(&format!("/api2/json/nodes/{node}/{kind}"), &fields)
            .await
            .and_then(Self::expect_upid)
    }

    /// `DELETE /nodes/{node}/{qemu,lxc}/{vmid}` -- delete a guest. Runs
    /// asynchronously; returns the task UPID.
    pub async fn delete_guest(&self, node: &str, kind: GuestKind, vmid: u32) -> Result<String> {
        self.delete(&format!("/api2/json/nodes/{node}/{kind}/{vmid}"))
            .await
            .and_then(Self::expect_upid)
    }

    /// `POST /nodes/{node}/{qemu,lxc}/{vmid}/clone` -- clone a guest.
    /// `fields` must include `newid`; common optional fields are `name`,
    /// `full` (full vs. linked clone), `target` (a different destination
    /// node), and `storage`. Runs asynchronously; returns the task UPID.
    pub async fn clone_guest(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        fields: Value,
    ) -> Result<String> {
        self.post_json(
            &format!("/api2/json/nodes/{node}/{kind}/{vmid}/clone"),
            &fields,
        )
        .await
        .and_then(Self::expect_upid)
    }

    /// `GET /nodes/{node}/{qemu,lxc}/{vmid}/snapshot` -- every snapshot
    /// taken of a guest, with its creation time and description.
    pub async fn list_snapshots(&self, node: &str, kind: GuestKind, vmid: u32) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/{kind}/{vmid}/snapshot"))
            .await
    }

    /// `POST /nodes/{node}/{qemu,lxc}/{vmid}/snapshot` -- create a
    /// snapshot. `fields` must include `snapname`; optional `description`,
    /// and for QEMU guests `vmstate` (also capture RAM state). Runs
    /// asynchronously; returns the task UPID.
    pub async fn create_snapshot(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        fields: Value,
    ) -> Result<String> {
        self.post_json(
            &format!("/api2/json/nodes/{node}/{kind}/{vmid}/snapshot"),
            &fields,
        )
        .await
        .and_then(Self::expect_upid)
    }

    /// `DELETE /nodes/{node}/{qemu,lxc}/{vmid}/snapshot/{snapname}` --
    /// delete a snapshot. Runs asynchronously; returns the task UPID.
    pub async fn delete_snapshot(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        snapname: &str,
    ) -> Result<String> {
        self.delete(&format!(
            "/api2/json/nodes/{node}/{kind}/{vmid}/snapshot/{snapname}"
        ))
        .await
        .and_then(Self::expect_upid)
    }

    /// `POST /nodes/{node}/{qemu,lxc}/{vmid}/snapshot/{snapname}/rollback`
    /// -- roll a guest back to a snapshot. Runs asynchronously; returns the
    /// task UPID.
    pub async fn rollback_snapshot(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        snapname: &str,
    ) -> Result<String> {
        self.post(&format!(
            "/api2/json/nodes/{node}/{kind}/{vmid}/snapshot/{snapname}/rollback"
        ))
        .await
        .and_then(Self::expect_upid)
    }

    /// `GET /cluster/resources` -- every resource in the cluster (nodes,
    /// guests, storage, SDN, pools) in one call, instead of paging through
    /// `list_nodes`/`list_guests` per node. `resource_type` filters to one
    /// kind (`"vm"`, `"storage"`, `"node"`, `"sdn"`, `"pool"`); `None`
    /// returns everything.
    pub async fn cluster_resources(&self, resource_type: Option<&str>) -> Result<Value> {
        match resource_type {
            Some(resource_type) => {
                self.get(&format!(
                    "/api2/json/cluster/resources?type={resource_type}"
                ))
                .await
            }
            None => self.get("/api2/json/cluster/resources").await,
        }
    }

    /// `GET /storage` -- every storage entry configured at the datacenter
    /// level (the shared config every node references), as opposed to
    /// [`ProxmoxClient::node_storage_status`]'s per-node usage/availability.
    pub async fn list_storage(&self) -> Result<Value> {
        self.get("/api2/json/storage").await
    }

    /// `GET /nodes/{node}/storage` -- status for every datastore visible
    /// from one node: usage, availability, and content types.
    pub async fn node_storage_status(&self, node: &str) -> Result<Value> {
        self.get(&format!("/api2/json/nodes/{node}/storage")).await
    }

    /// `GET /cluster/backup` -- every scheduled vzdump backup job.
    pub async fn list_backup_jobs(&self) -> Result<Value> {
        self.get("/api2/json/cluster/backup").await
    }

    /// `POST /nodes/{node}/vzdump` -- run a backup immediately, outside any
    /// schedule. `fields` is the same field set Proxmox's own vzdump API
    /// takes (`vmid`, `storage`, `mode`, `compress`, `all`, ...) -- passed
    /// through as-is, since valid combinations vary widely (a single guest
    /// vs. every guest, snapshot vs. stop mode, ...). Runs asynchronously;
    /// returns the task UPID.
    pub async fn run_backup(&self, node: &str, fields: Value) -> Result<String> {
        self.post_json(&format!("/api2/json/nodes/{node}/vzdump"), &fields)
            .await
            .and_then(Self::expect_upid)
    }

    /// `POST /nodes/{node}/{qemu,lxc}/{vmid}/migrate` -- migrate a guest to
    /// another node. `fields` must include `target` (the destination
    /// node); common optional fields are `online` (live-migrate a running
    /// QEMU guest instead of suspending it), `bwlimit`, and
    /// `with-local-disks`/`targetstorage` (QEMU) or
    /// `restart`/`target-storage` (LXC). Runs asynchronously; returns the
    /// task UPID.
    pub async fn migrate_guest(
        &self,
        node: &str,
        kind: GuestKind,
        vmid: u32,
        fields: Value,
    ) -> Result<String> {
        self.post_json(
            &format!("/api2/json/nodes/{node}/{kind}/{vmid}/migrate"),
            &fields,
        )
        .await
        .and_then(Self::expect_upid)
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

    async fn post_json(&self, path: &str, payload: &Value) -> Result<Value> {
        let response = self
            .http
            .post(&format!("{}{path}", self.base_url))?
            .header("Authorization", &self.auth_header)?
            .header("Content-Type", "application/json")?
            .body(serde_json::to_string(payload)?)
            .send()
            .await?;
        Self::unwrap_data(response).await
    }

    async fn put_json(&self, path: &str, payload: &Value) -> Result<Value> {
        let response = self
            .http
            .put(&format!("{}{path}", self.base_url))?
            .header("Authorization", &self.auth_header)?
            .header("Content-Type", "application/json")?
            .body(serde_json::to_string(payload)?)
            .send()
            .await?;
        Self::unwrap_data(response).await
    }

    async fn delete(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .delete(&format!("{}{path}", self.base_url))?
            .header("Authorization", &self.auth_header)?
            .send()
            .await?;
        Self::unwrap_data(response).await
    }

    /// Every asynchronous Proxmox action (guest power, create/delete/clone,
    /// snapshot create/delete/rollback) returns its task ID as a bare
    /// `data` string.
    fn expect_upid(data: Value) -> Result<String> {
        data.as_str().map(str::to_string).ok_or_else(|| {
            Error::MissingData(format!("expected a UPID string in `data`, got {data}"))
        })
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

//! Proxmox VE tools: node/guest listing, status, power control, config,
//! lifecycle (create/delete/clone/migrate), snapshots, cluster resources,
//! storage, and backups.

use rmcp::{Json, handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use rusty_mcp::ToolError;
use rusty_proxmox::{GuestKind, PowerAction};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::json_result::JsonResult;
use crate::server::HomelabServer;

/// Which kind of Proxmox guest a tool call is about.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GuestKindArg {
    /// A QEMU/KVM virtual machine.
    Qemu,
    /// An LXC container.
    Lxc,
}

impl From<GuestKindArg> for GuestKind {
    fn from(value: GuestKindArg) -> Self {
        match value {
            GuestKindArg::Qemu => GuestKind::Qemu,
            GuestKindArg::Lxc => GuestKind::Lxc,
        }
    }
}

/// A power action to perform on a guest.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PowerActionArg {
    /// Start the guest.
    Start,
    /// Hard stop -- pull the power cord, no clean shutdown inside the guest.
    Stop,
    /// Ask the guest to shut down cleanly.
    Shutdown,
    /// Reboot the guest.
    Reboot,
    /// Suspend the guest to disk/RAM.
    Suspend,
    /// Resume a suspended guest.
    Resume,
}

impl From<PowerActionArg> for PowerAction {
    fn from(value: PowerActionArg) -> Self {
        match value {
            PowerActionArg::Start => PowerAction::Start,
            PowerActionArg::Stop => PowerAction::Stop,
            PowerActionArg::Shutdown => PowerAction::Shutdown,
            PowerActionArg::Reboot => PowerAction::Reboot,
            PowerActionArg::Suspend => PowerAction::Suspend,
            PowerActionArg::Resume => PowerAction::Resume,
        }
    }
}

/// Arguments naming a Proxmox node.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
}

/// Arguments naming a node and a guest kind.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GuestListArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest to list.
    pub kind: GuestKindArg,
}

/// Arguments naming one specific guest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GuestArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
}

/// Arguments naming a Proxmox task by its UPID.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskArgs {
    /// Node name the task is running on, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// The task's UPID, as returned by `proxmox_guest_power` (or any other
    /// asynchronous action).
    pub upid: String,
}

/// Arguments for a guest power action.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GuestPowerArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The power action to perform.
    pub action: PowerActionArg,
}

/// Arguments for updating a guest's configuration.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateGuestConfigArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The fields to update, in the same shape Proxmox's own config API
    /// takes (`cores`, `memory`, `net0`, `scsi0`, ...). Call
    /// `proxmox_guest_config` first to see the guest's current fields.
    pub config: serde_json::Value,
}

/// Arguments for creating a guest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateGuestArgs {
    /// Node to create the guest on, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest to create.
    pub kind: GuestKindArg,
    /// The new guest's fields. Must include `vmid`; the rest is the same
    /// field set Proxmox's own create-guest API takes, and differs between
    /// QEMU (`ostype`, `scsi0`, `net0`, ...) and LXC (`ostemplate`,
    /// `rootfs`, ...).
    pub config: serde_json::Value,
}

/// Arguments for cloning a guest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloneGuestArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The source guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The clone's fields. Must include `newid`; common optional fields are
    /// `name`, `full` (full vs. linked clone), `target` (a different
    /// destination node), and `storage`.
    pub config: serde_json::Value,
}

/// Arguments naming one snapshot of a guest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The snapshot's name, as returned by `proxmox_list_snapshots`.
    pub snapname: String,
}

/// Arguments for creating a snapshot.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSnapshotArgs {
    /// Node name, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The snapshot's fields. Must include `snapname`; optional
    /// `description`, and for QEMU guests `vmstate` (also capture RAM
    /// state).
    pub snapshot: serde_json::Value,
}

/// Which kind of resource to filter a cluster resources overview to.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClusterResourceTypeArg {
    /// QEMU VMs and LXC containers.
    Vm,
    /// Storage entries.
    Storage,
    /// Cluster nodes.
    Node,
    /// SDN objects.
    Sdn,
    /// Resource pools.
    Pool,
}

impl ClusterResourceTypeArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vm => "vm",
            Self::Storage => "storage",
            Self::Node => "node",
            Self::Sdn => "sdn",
            Self::Pool => "pool",
        }
    }
}

/// Arguments for the cluster resources overview.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClusterResourcesArgs {
    /// Restrict results to one kind of resource. Omit for everything.
    #[serde(default)]
    pub resource_type: Option<ClusterResourceTypeArg>,
}

/// Arguments for running an on-demand backup.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunBackupArgs {
    /// Node to run the backup from, as returned by `proxmox_list_nodes`.
    pub node: String,
    /// The backup's fields, in the same shape Proxmox's own vzdump API
    /// takes: `vmid` (one guest, or omit for every guest with `all: true`),
    /// `storage`, `mode` (`snapshot`/`suspend`/`stop`), `compress`, and so
    /// on.
    pub backup: serde_json::Value,
}

/// Arguments for migrating a guest to another node.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MigrateGuestArgs {
    /// Node the guest currently runs on, as returned by
    /// `proxmox_list_nodes`.
    pub node: String,
    /// Which kind of guest `vmid` is.
    pub kind: GuestKindArg,
    /// The guest's VMID, as returned by `proxmox_list_guests`.
    pub vmid: u32,
    /// The migration's fields. Must include `target` (the destination
    /// node); common optional fields are `online` (live-migrate a running
    /// QEMU guest instead of suspending it) and `bwlimit`.
    pub migration: serde_json::Value,
}

#[tool_router(router = proxmox_tools, vis = "pub(crate)")]
impl HomelabServer {
    /// List Proxmox cluster nodes and their status.
    #[tool(
        description = "List every node in the Proxmox cluster, with its online/offline status, CPU and memory usage, and uptime."
    )]
    pub async fn proxmox_list_nodes(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .list_nodes()
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// One node's detailed status.
    #[tool(
        description = "Get one Proxmox node's detailed status: CPU, memory, swap, load average, kernel version, and uptime. Call proxmox_list_nodes first to find node names."
    )]
    pub async fn proxmox_node_status(
        &self,
        Parameters(NodeArgs { node }): Parameters<NodeArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .node_status(&node)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// List guests of one kind on a node.
    #[tool(
        description = "List every QEMU virtual machine or LXC container on a Proxmox node, with its VMID, name, and run status. Call proxmox_list_nodes first to find node names."
    )]
    pub async fn proxmox_list_guests(
        &self,
        Parameters(GuestListArgs { node, kind }): Parameters<GuestListArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .list_guests(&node, kind.into())
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// One guest's live status.
    #[tool(
        description = "Get one Proxmox guest's live status: run state, uptime, CPU/memory usage, and configured resources. Call proxmox_list_guests first to find VMIDs."
    )]
    pub async fn proxmox_guest_status(
        &self,
        Parameters(GuestArgs { node, kind, vmid }): Parameters<GuestArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .guest_status(&node, kind.into(), vmid)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Start, stop, shut down, reboot, suspend, or resume a guest.
    #[tool(
        description = "Start, stop, cleanly shut down, reboot, suspend, or resume a Proxmox guest. Runs asynchronously on the Proxmox side; returns the task ID (a UPID string) rather than waiting for the action to finish."
    )]
    pub async fn proxmox_guest_power(
        &self,
        Parameters(GuestPowerArgs {
            node,
            kind,
            vmid,
            action,
        }): Parameters<GuestPowerArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .guest_power(&node, kind.into(), vmid, action.into())
            .await
            .map_err(proxmox_error)
    }

    /// Whether an asynchronous task has finished, and if so, whether it
    /// succeeded.
    #[tool(
        description = "Check whether an asynchronous Proxmox task (identified by the UPID that proxmox_guest_power or another action returned) has finished, and if so, whether it succeeded."
    )]
    pub async fn proxmox_task_status(
        &self,
        Parameters(TaskArgs { node, upid }): Parameters<TaskArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .task_status(&node, &upid)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// An asynchronous task's log output.
    #[tool(
        description = "Get an asynchronous Proxmox task's log output (identified by the UPID that proxmox_guest_power or another action returned) -- most useful for finding out why a task failed."
    )]
    pub async fn proxmox_task_log(
        &self,
        Parameters(TaskArgs { node, upid }): Parameters<TaskArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .task_log(&node, &upid)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// One guest's current configuration.
    #[tool(
        description = "Get one Proxmox guest's current configuration: CPU, memory, disks, network interfaces, boot order, and everything else Proxmox stores per-guest. Call proxmox_list_guests first to find VMIDs."
    )]
    pub async fn proxmox_guest_config(
        &self,
        Parameters(GuestArgs { node, kind, vmid }): Parameters<GuestArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .guest_config(&node, kind.into(), vmid)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Update a guest's configuration.
    #[tool(
        description = "Update a Proxmox guest's configuration (CPU, memory, disks, network interfaces, ...). Usually takes effect immediately, but some fields (e.g. a disk resize) run as a background task instead -- check whether the result looks like a UPID string."
    )]
    pub async fn proxmox_update_guest_config(
        &self,
        Parameters(UpdateGuestConfigArgs {
            node,
            kind,
            vmid,
            config,
        }): Parameters<UpdateGuestConfigArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .update_guest_config(&node, kind.into(), vmid, config)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Create a new guest.
    #[tool(
        description = "Create a new Proxmox QEMU virtual machine or LXC container. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish."
    )]
    pub async fn proxmox_create_guest(
        &self,
        Parameters(CreateGuestArgs { node, kind, config }): Parameters<CreateGuestArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .create_guest(&node, kind.into(), config)
            .await
            .map_err(proxmox_error)
    }

    /// Delete a guest.
    #[tool(
        description = "Delete a Proxmox guest. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish. Call proxmox_list_guests first to find VMIDs."
    )]
    pub async fn proxmox_delete_guest(
        &self,
        Parameters(GuestArgs { node, kind, vmid }): Parameters<GuestArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .delete_guest(&node, kind.into(), vmid)
            .await
            .map_err(proxmox_error)
    }

    /// Clone a guest.
    #[tool(
        description = "Clone a Proxmox guest into a new one. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish. Call proxmox_list_guests first to find the source VMID."
    )]
    pub async fn proxmox_clone_guest(
        &self,
        Parameters(CloneGuestArgs {
            node,
            kind,
            vmid,
            config,
        }): Parameters<CloneGuestArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .clone_guest(&node, kind.into(), vmid, config)
            .await
            .map_err(proxmox_error)
    }

    /// Every snapshot of a guest.
    #[tool(
        description = "List every snapshot taken of a Proxmox guest, with its creation time and description. Call proxmox_list_guests first to find VMIDs."
    )]
    pub async fn proxmox_list_snapshots(
        &self,
        Parameters(GuestArgs { node, kind, vmid }): Parameters<GuestArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .list_snapshots(&node, kind.into(), vmid)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Create a snapshot.
    #[tool(
        description = "Create a snapshot of a Proxmox guest. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish."
    )]
    pub async fn proxmox_create_snapshot(
        &self,
        Parameters(CreateSnapshotArgs {
            node,
            kind,
            vmid,
            snapshot,
        }): Parameters<CreateSnapshotArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .create_snapshot(&node, kind.into(), vmid, snapshot)
            .await
            .map_err(proxmox_error)
    }

    /// Delete a snapshot.
    #[tool(
        description = "Delete a snapshot of a Proxmox guest. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish. Call proxmox_list_snapshots first to find snapshot names."
    )]
    pub async fn proxmox_delete_snapshot(
        &self,
        Parameters(SnapshotArgs {
            node,
            kind,
            vmid,
            snapname,
        }): Parameters<SnapshotArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .delete_snapshot(&node, kind.into(), vmid, &snapname)
            .await
            .map_err(proxmox_error)
    }

    /// Roll a guest back to a snapshot.
    #[tool(
        description = "Roll a Proxmox guest back to a previous snapshot, discarding any changes made since. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish. Call proxmox_list_snapshots first to find snapshot names."
    )]
    pub async fn proxmox_rollback_snapshot(
        &self,
        Parameters(SnapshotArgs {
            node,
            kind,
            vmid,
            snapname,
        }): Parameters<SnapshotArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .rollback_snapshot(&node, kind.into(), vmid, &snapname)
            .await
            .map_err(proxmox_error)
    }

    /// Every resource in the cluster, in one call.
    #[tool(
        description = "List every resource in the Proxmox cluster (nodes, guests, storage, SDN, pools) in one call, instead of paging through proxmox_list_nodes/proxmox_list_guests per node. Pass resource_type to restrict to one kind."
    )]
    pub async fn proxmox_cluster_resources(
        &self,
        Parameters(ClusterResourcesArgs { resource_type }): Parameters<ClusterResourcesArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .cluster_resources(resource_type.map(ClusterResourceTypeArg::as_str))
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Every storage entry configured at the datacenter level.
    #[tool(
        description = "List every storage entry configured at the Proxmox datacenter level (the shared config every node references)."
    )]
    pub async fn proxmox_list_storage(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .list_storage()
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Storage status for one node.
    #[tool(
        description = "Get usage, availability, and content types for every datastore visible from one Proxmox node. Call proxmox_list_nodes first to find node names."
    )]
    pub async fn proxmox_node_storage_status(
        &self,
        Parameters(NodeArgs { node }): Parameters<NodeArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .node_storage_status(&node)
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Every scheduled backup job.
    #[tool(
        description = "List every scheduled vzdump backup job configured on the Proxmox cluster."
    )]
    pub async fn proxmox_list_backup_jobs(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.proxmox()?
                .list_backup_jobs()
                .await
                .map_err(proxmox_error)?
                .into(),
        ))
    }

    /// Run a backup immediately.
    #[tool(
        description = "Run a Proxmox backup immediately, outside any schedule. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish."
    )]
    pub async fn proxmox_run_backup(
        &self,
        Parameters(RunBackupArgs { node, backup }): Parameters<RunBackupArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .run_backup(&node, backup)
            .await
            .map_err(proxmox_error)
    }

    /// Migrate a guest to another node.
    #[tool(
        description = "Migrate a Proxmox guest to another cluster node. Runs asynchronously; returns the task ID (a UPID string) rather than waiting for it to finish. Call proxmox_list_guests first to find the VMID and proxmox_list_nodes for the destination node name."
    )]
    pub async fn proxmox_migrate_guest(
        &self,
        Parameters(MigrateGuestArgs {
            node,
            kind,
            vmid,
            migration,
        }): Parameters<MigrateGuestArgs>,
    ) -> Result<String, ErrorData> {
        self.proxmox()?
            .migrate_guest(&node, kind.into(), vmid, migration)
            .await
            .map_err(proxmox_error)
    }
}

/// Turns a client-side failure into a protocol error the model can see and
/// reason about (bad node name, expired token, unreachable host, ...).
fn proxmox_error(err: rusty_proxmox::Error) -> ErrorData {
    ToolError::failed(err.to_string()).into()
}

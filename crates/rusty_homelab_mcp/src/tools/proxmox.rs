//! Proxmox VE tools: node/guest listing, status, and power control.

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
}

/// Turns a client-side failure into a protocol error the model can see and
/// reason about (bad node name, expired token, unreachable host, ...).
fn proxmox_error(err: rusty_proxmox::Error) -> ErrorData {
    ToolError::failed(err.to_string()).into()
}

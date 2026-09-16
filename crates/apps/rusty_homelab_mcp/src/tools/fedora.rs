//! Fedora tools: system status, systemd service listing/control, journal
//! reads, dnf update listing/install/remove (with task polling), and
//! allowlisted config file read/write -- against a
//! [`rusty_fedora_agent`](https://docs.rs/rusty_fedora_agent) instance
//! running on the managed Fedora host, via the [`rusty_fedora`] typed
//! client. Follows the same discovery-then-mutate pattern as the
//! OPNsense/Proxmox tools (`fedora_list_services` before
//! `fedora_service_control`, `fedora_dnf_list_updates` before
//! `fedora_dnf_install`).

use rmcp::{Json, handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use rusty_fedora::{Priority, ServiceAction, UnitType};
use rusty_mcp::ToolError;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::json_result::JsonResult;
use crate::server::HomelabServer;

/// Which systemd unit type to list.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UnitTypeArg {
    /// A `.service` unit.
    Service,
    /// A `.timer` unit.
    Timer,
    /// A `.socket` unit.
    Socket,
}

impl From<UnitTypeArg> for UnitType {
    fn from(value: UnitTypeArg) -> Self {
        match value {
            UnitTypeArg::Service => UnitType::Service,
            UnitTypeArg::Timer => UnitType::Timer,
            UnitTypeArg::Socket => UnitType::Socket,
        }
    }
}

/// An action to take on a named systemd unit.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ServiceActionArg {
    /// Start the unit.
    Start,
    /// Stop the unit.
    Stop,
    /// Restart the unit.
    Restart,
    /// Enable the unit (start automatically at boot).
    Enable,
    /// Disable the unit.
    Disable,
}

impl From<ServiceActionArg> for ServiceAction {
    fn from(value: ServiceActionArg) -> Self {
        match value {
            ServiceActionArg::Start => ServiceAction::Start,
            ServiceActionArg::Stop => ServiceAction::Stop,
            ServiceActionArg::Restart => ServiceAction::Restart,
            ServiceActionArg::Enable => ServiceAction::Enable,
            ServiceActionArg::Disable => ServiceAction::Disable,
        }
    }
}

/// A `journalctl -p` priority filter.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PriorityArg {
    Emerg,
    Alert,
    Crit,
    Err,
    Warning,
    Notice,
    Info,
    Debug,
}

impl From<PriorityArg> for Priority {
    fn from(value: PriorityArg) -> Self {
        match value {
            PriorityArg::Emerg => Priority::Emerg,
            PriorityArg::Alert => Priority::Alert,
            PriorityArg::Crit => Priority::Crit,
            PriorityArg::Err => Priority::Err,
            PriorityArg::Warning => Priority::Warning,
            PriorityArg::Notice => Priority::Notice,
            PriorityArg::Info => Priority::Info,
            PriorityArg::Debug => Priority::Debug,
        }
    }
}

/// Arguments carrying only an optional host selector.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HostArgs {
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments for listing services.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListServicesArgs {
    /// Restrict to one unit type. Omit to list services, timers, and
    /// sockets together.
    #[serde(default)]
    pub unit_type: Option<UnitTypeArg>,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments for a service control call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FedoraServiceControlArgs {
    /// The unit's name, as returned by `fedora_list_services` (e.g.
    /// `ollama.service`). Must be in the agent's unit allowlist.
    pub name: String,
    /// The action to perform.
    pub action: ServiceActionArg,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments for reading the journal.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadJournalArgs {
    /// Restrict to one unit's log lines, as returned by
    /// `fedora_list_services`. Omit to read the full system journal.
    #[serde(default)]
    pub unit: Option<String>,
    /// How many of the most recent lines to return. Defaults to 100.
    #[serde(default)]
    pub lines: Option<u32>,
    /// Only lines at or after this time, in any format `journalctl
    /// --since` accepts (e.g. `"2026-09-04 08:00:00"`, `"1 hour ago"`,
    /// `"yesterday"`).
    #[serde(default)]
    pub since: Option<String>,
    /// Only lines at or above this severity.
    #[serde(default)]
    pub priority: Option<PriorityArg>,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments naming a set of packages.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PackagesArgs {
    /// Exact package names, as returned by `fedora_dnf_list_updates` or
    /// otherwise known ahead of time. Must all be in the agent's package
    /// allowlist -- the whole call is refused if any one isn't.
    pub packages: Vec<String>,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments naming a task by id.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskIdArgs {
    /// The task id, as returned by `fedora_dnf_install`/`fedora_dnf_remove`.
    pub task_id: String,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments for reading a config file.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadConfigArgs {
    /// Absolute path to the config file. Must be under one of the
    /// agent's allowlisted config-path prefixes.
    pub path: String,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

/// Arguments for writing a config file.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteConfigArgs {
    /// Absolute path to the config file. Must be under one of the
    /// agent's allowlisted config-path prefixes.
    pub path: String,
    /// The file's new full content -- replaces whatever was there.
    pub content: String,
    /// Write a `.bak` copy of the previous content first, if the file
    /// already exists. Defaults to `true`.
    #[serde(default)]
    pub backup: Option<bool>,
    /// Which managed Fedora host to target, e.g. "samba-lxc-101", as
    /// configured on the server via `--fedora-hosts-file`. Defaults to
    /// "baileyai".
    #[serde(default)]
    pub host: Option<String>,
}

#[tool_router(router = fedora_tools, vis = "pub(crate)")]
impl HomelabServer {
    /// Uptime, load average, memory, kernel/OS release.
    #[tool(
        description = "Get the managed Fedora host's overall system status: uptime, load average, memory, and kernel/OS release."
    )]
    pub async fn fedora_system_status(
        &self,
        Parameters(HostArgs { host }): Parameters<HostArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .system_status()
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Every unit of the given type(s), with its running state.
    #[tool(
        description = "List systemd units (services, timers, and/or sockets) on the managed Fedora host, with their load/active/sub state and short id (used by fedora_service_control)."
    )]
    pub async fn fedora_list_services(
        &self,
        Parameters(ListServicesArgs { unit_type, host }): Parameters<ListServicesArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .list_services(unit_type.map(Into::into))
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Start, stop, restart, enable, or disable a unit.
    #[tool(
        description = "Start, stop, restart, enable, or disable a named systemd unit on the managed Fedora host. Call fedora_list_services first to find valid unit names -- the agent additionally refuses any unit not in its own allowlist."
    )]
    pub async fn fedora_service_control(
        &self,
        Parameters(FedoraServiceControlArgs { name, action, host }): Parameters<
            FedoraServiceControlArgs,
        >,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .service_control(&name, action.into())
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Journal lines, most recent last.
    #[tool(
        description = "Read journal lines from the managed Fedora host, most recent last. Omit unit for the full system journal; defaults to the last 100 lines."
    )]
    pub async fn fedora_read_journal(
        &self,
        Parameters(ReadJournalArgs {
            unit,
            lines,
            since,
            priority,
            host,
        }): Parameters<ReadJournalArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .read_journal(
                    unit.as_deref(),
                    lines,
                    since.as_deref(),
                    priority.map(Into::into),
                )
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Every package with an update available.
    #[tool(
        description = "List every package with a dnf update available on the managed Fedora host. Call this before fedora_dnf_install to upgrade something."
    )]
    pub async fn fedora_dnf_list_updates(
        &self,
        Parameters(HostArgs { host }): Parameters<HostArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .dnf_list_updates()
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Install packages; returns a task id.
    #[tool(
        description = "Install packages via dnf on the managed Fedora host. Installs can run long, so this returns a task id immediately -- poll fedora_task_status with it. The agent refuses the whole call if any package isn't in its own allowlist."
    )]
    pub async fn fedora_dnf_install(
        &self,
        Parameters(PackagesArgs { packages, host }): Parameters<PackagesArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .dnf_install(&packages)
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Remove packages; returns a task id.
    #[tool(
        description = "Remove packages via dnf on the managed Fedora host. Same asynchronous-task shape as fedora_dnf_install -- poll fedora_task_status with the returned task id. The agent refuses the whole call if any package isn't in its own allowlist."
    )]
    pub async fn fedora_dnf_remove(
        &self,
        Parameters(PackagesArgs { packages, host }): Parameters<PackagesArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .dnf_remove(&packages)
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// A dnf install/remove task's current state.
    #[tool(
        description = "Get a dnf install/remove task's current state (running/succeeded/failed) on the managed Fedora host, with stdout/stderr/exit_code once it's finished. Call fedora_dnf_install or fedora_dnf_remove first to get a task id."
    )]
    pub async fn fedora_task_status(
        &self,
        Parameters(TaskIdArgs { task_id, host }): Parameters<TaskIdArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .task_status(&task_id)
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// A config file's raw content.
    #[tool(
        description = "Read a config file's raw content from the managed Fedora host. The agent refuses any path outside its own allowlisted config-path prefixes."
    )]
    pub async fn fedora_read_config(
        &self,
        Parameters(ReadConfigArgs { path, host }): Parameters<ReadConfigArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .read_config(&path)
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }

    /// Replace a config file's content.
    #[tool(
        description = "Replace a config file's full content on the managed Fedora host. Writes a .bak copy of the previous content first by default. The agent refuses any path outside its own allowlisted config-path prefixes."
    )]
    pub async fn fedora_write_config(
        &self,
        Parameters(WriteConfigArgs {
            path,
            content,
            backup,
            host,
        }): Parameters<WriteConfigArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.fedora(host.as_deref())?
                .write_config(&path, &content, backup.unwrap_or(true))
                .await
                .map_err(fedora_error)?
                .into(),
        ))
    }
}

/// Turns a client-side failure into a protocol error the model can see and
/// reason about (an allowlist rejection, an unreachable agent, ...).
fn fedora_error(err: rusty_fedora::Error) -> ErrorData {
    ToolError::failed(err.to_string()).into()
}

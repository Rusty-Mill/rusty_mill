//! OPNsense tools: system status, service control, interfaces, firewall
//! aliases/rules, gateways, DHCP leases, VLANs, ARP/routing diagnostics,
//! and config backup/restore.

use crate::tool_support::{ErrorData, Json, NoArgs, Parameters, ToolError, register};
use rusty_opnsense::ServiceAction;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::json_result::JsonResult;
use crate::server::HomelabServer;

/// An action to take on a named service.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ServiceActionArg {
    /// Start the service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service.
    Restart,
}

impl From<ServiceActionArg> for ServiceAction {
    fn from(value: ServiceActionArg) -> Self {
        match value {
            ServiceActionArg::Start => ServiceAction::Start,
            ServiceActionArg::Stop => ServiceAction::Stop,
            ServiceActionArg::Restart => ServiceAction::Restart,
        }
    }
}

/// Arguments for a service control call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ServiceControlArgs {
    /// The service's short id, as returned by `opnsense_list_services`
    /// (e.g. `unbound`, `dhcpd`, `sshd`).
    pub name: String,
    /// The action to perform.
    pub action: ServiceActionArg,
}

/// Arguments naming a firewall rule by UUID.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FirewallRuleUuidArgs {
    /// The rule's UUID, as returned by `opnsense_list_firewall_rules` or
    /// `opnsense_create_firewall_rule`.
    pub uuid: String,
}

/// Arguments for creating a firewall rule.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateFirewallRuleArgs {
    /// The rule's fields, in the same shape OPNsense's own rule form
    /// submits: `action` (pass/block/reject), `interface`, `direction`
    /// (in/out), `protocol`, `source_net`, `source_port`,
    /// `destination_net`, `destination_port`, `description`, `log`,
    /// `enabled`, and so on. Call `opnsense_list_firewall_rules` first to
    /// see example field names and values from existing rules.
    pub rule: serde_json::Value,
}

/// Arguments for updating a firewall rule.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateFirewallRuleArgs {
    /// The rule's UUID, as returned by `opnsense_list_firewall_rules`.
    pub uuid: String,
    /// The rule's new field set -- replaces the rule entirely, so call
    /// `opnsense_get_firewall_rule` first and send back its full field set
    /// unless clearing the fields left out is intended.
    pub rule: serde_json::Value,
}

/// Arguments for toggling a firewall rule's enabled state.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ToggleFirewallRuleArgs {
    /// The rule's UUID, as returned by `opnsense_list_firewall_rules`.
    pub uuid: String,
    /// Set the rule's enabled state explicitly. Omit to just flip whatever
    /// it currently is.
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Arguments naming a VLAN by UUID.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VlanUuidArgs {
    /// The VLAN's UUID, as returned by `opnsense_list_vlans` or
    /// `opnsense_create_vlan`.
    pub uuid: String,
}

/// Arguments for creating a VLAN.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateVlanArgs {
    /// The VLAN's fields, in the same shape OPNsense's own VLAN form
    /// submits: `if` (the parent interface), `tag`, `descr`, `pcp`. Call
    /// `opnsense_list_vlans` first to see example field names and values
    /// from existing VLANs.
    pub vlan: serde_json::Value,
}

/// Arguments for updating a VLAN.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateVlanArgs {
    /// The VLAN's UUID, as returned by `opnsense_list_vlans`.
    pub uuid: String,
    /// The VLAN's new field set -- replaces it entirely, so call
    /// `opnsense_get_vlan` first and send back its full field set unless
    /// clearing the fields left out is intended.
    pub vlan: serde_json::Value,
}

/// Arguments naming a backup provider.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackupHostArgs {
    /// The backup provider's id, as returned by
    /// `opnsense_list_backup_providers` (every install has at least
    /// `this`, the firewall's own local config history).
    pub host: String,
}

/// Arguments for downloading a backup.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadBackupArgs {
    /// The backup provider's id, as returned by
    /// `opnsense_list_backup_providers`.
    pub host: String,
    /// Which backup to download, as returned by `opnsense_list_backups`.
    /// Omit to download the current running config instead of a
    /// previously saved backup.
    #[serde(default)]
    pub backup: Option<String>,
}

/// Arguments for restoring a backup.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestoreBackupArgs {
    /// The backup to restore, as returned by `opnsense_list_backups`.
    pub backup: String,
}

impl HomelabServer {
    /// Firmware version, running kernel, pending updates, service health.
    pub async fn opnsense_system_status(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .system_status()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every known service and its running state.
    pub async fn opnsense_list_services(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_services()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Start, stop, or restart a service.
    pub async fn opnsense_service_control(
        &self,
        Parameters(ServiceControlArgs { name, action }): Parameters<ServiceControlArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .service_control(&name, action.into())
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every network interface OPNsense knows about.
    pub async fn opnsense_list_interfaces(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_interfaces()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every configured firewall alias.
    pub async fn opnsense_list_firewall_aliases(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_firewall_aliases()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every configured gateway and its monitor status.
    pub async fn opnsense_list_gateways(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_gateways()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every configured firewall rule.
    pub async fn opnsense_list_firewall_rules(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_firewall_rules()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// One firewall rule's full field set.
    pub async fn opnsense_get_firewall_rule(
        &self,
        Parameters(FirewallRuleUuidArgs { uuid }): Parameters<FirewallRuleUuidArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .get_firewall_rule(&uuid)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Create a firewall rule.
    pub async fn opnsense_create_firewall_rule(
        &self,
        Parameters(CreateFirewallRuleArgs { rule }): Parameters<CreateFirewallRuleArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .create_firewall_rule(rule)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Update a firewall rule.
    pub async fn opnsense_update_firewall_rule(
        &self,
        Parameters(UpdateFirewallRuleArgs { uuid, rule }): Parameters<UpdateFirewallRuleArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .update_firewall_rule(&uuid, rule)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Delete a firewall rule.
    pub async fn opnsense_delete_firewall_rule(
        &self,
        Parameters(FirewallRuleUuidArgs { uuid }): Parameters<FirewallRuleUuidArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .delete_firewall_rule(&uuid)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Enable or disable a firewall rule.
    pub async fn opnsense_toggle_firewall_rule(
        &self,
        Parameters(ToggleFirewallRuleArgs { uuid, enabled }): Parameters<ToggleFirewallRuleArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .toggle_firewall_rule(&uuid, enabled)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Apply pending firewall changes.
    pub async fn opnsense_apply_firewall_changes(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .apply_firewall_changes()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every current DHCP lease.
    pub async fn opnsense_list_dhcp_leases(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_dhcp_leases()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every configured VLAN.
    pub async fn opnsense_list_vlans(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_vlans()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// One VLAN's full field set.
    pub async fn opnsense_get_vlan(
        &self,
        Parameters(VlanUuidArgs { uuid }): Parameters<VlanUuidArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .get_vlan(&uuid)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Create a VLAN.
    pub async fn opnsense_create_vlan(
        &self,
        Parameters(CreateVlanArgs { vlan }): Parameters<CreateVlanArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .create_vlan(vlan)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Update a VLAN.
    pub async fn opnsense_update_vlan(
        &self,
        Parameters(UpdateVlanArgs { uuid, vlan }): Parameters<UpdateVlanArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .update_vlan(&uuid, vlan)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Delete a VLAN.
    pub async fn opnsense_delete_vlan(
        &self,
        Parameters(VlanUuidArgs { uuid }): Parameters<VlanUuidArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .delete_vlan(&uuid)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Apply pending VLAN changes.
    pub async fn opnsense_apply_vlan_changes(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .apply_vlan_changes()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// The current ARP table.
    pub async fn opnsense_list_arp_entries(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_arp_entries()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// The system routing table.
    pub async fn opnsense_list_routes(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_routes()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every configured backup provider.
    pub async fn opnsense_list_backup_providers(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_backup_providers()
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// Every backup available from one provider.
    pub async fn opnsense_list_backups(
        &self,
        Parameters(BackupHostArgs { host }): Parameters<BackupHostArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_backups(&host)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }

    /// One backup's raw config.xml content.
    pub async fn opnsense_download_backup(
        &self,
        Parameters(DownloadBackupArgs { host, backup }): Parameters<DownloadBackupArgs>,
    ) -> Result<String, ErrorData> {
        self.opnsense()?
            .download_backup(&host, backup.as_deref())
            .await
            .map_err(opnsense_error)
    }

    /// Revert to a previous backup.
    pub async fn opnsense_restore_backup(
        &self,
        Parameters(RestoreBackupArgs { backup }): Parameters<RestoreBackupArgs>,
    ) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .restore_backup(&backup)
                .await
                .map_err(opnsense_error)?
                .into(),
        ))
    }
}

/// Turns a client-side failure into a protocol error the model can see and
/// reason about (bad service name, expired key, unreachable host, ...).
fn opnsense_error(err: rusty_opnsense::Error) -> ErrorData {
    ToolError::failed(err.to_string()).into()
}

/// Offer every tool of this module on `builder`.
pub(crate) fn register_tools(
    builder: rusty_mcp_server::ServerBuilder,
    server: &HomelabServer,
    rt: &tokio::runtime::Handle,
) -> rusty_mcp_server::ServerBuilder {
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_system_status",
        "Get the OPNsense firewall's overall system status: firmware version, running kernel, pending updates, and per-service health.",
        |s, _a| async move { s.opnsense_system_status().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_services",
        "List every service OPNsense's service supervisor knows about, with its running state and short id (used by opnsense_service_control).",
        |s, _a| async move { s.opnsense_list_services().await },
    );
    let builder = register::<ServiceControlArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_service_control",
        "Start, stop, or restart a named OPNsense service. Call opnsense_list_services first to find valid service ids.",
        |s, a| async move { s.opnsense_service_control(Parameters(a)).await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_interfaces",
        "List every network interface OPNsense knows about, keyed by device name.",
        |s, _a| async move { s.opnsense_list_interfaces().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_firewall_aliases",
        "List every firewall alias currently configured on OPNsense (name, type, and contents).",
        |s, _a| async move { s.opnsense_list_firewall_aliases().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_gateways",
        "List every configured gateway on OPNsense with its monitor status (none/loss/down) and ping latency.",
        |s, _a| async move { s.opnsense_list_gateways().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_firewall_rules",
        "List every firewall rule currently configured on OPNsense, with its UUID, enabled state, action, interface, direction, protocol, and source/destination. Call this to find a rule's UUID before getting, updating, deleting, or toggling it.",
        |s, _a| async move { s.opnsense_list_firewall_rules().await },
    );
    let builder = register::<FirewallRuleUuidArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_get_firewall_rule",
        "Get one OPNsense firewall rule's full field set by UUID. Call opnsense_list_firewall_rules first to find UUIDs.",
        |s, a| async move { s.opnsense_get_firewall_rule(Parameters(a)).await },
    );
    let builder = register::<CreateFirewallRuleArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_create_firewall_rule",
        "Create a new OPNsense firewall rule. Does not take effect until opnsense_apply_firewall_changes is called.",
        |s, a| async move { s.opnsense_create_firewall_rule(Parameters(a)).await },
    );
    let builder = register::<UpdateFirewallRuleArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_update_firewall_rule",
        "Update an existing OPNsense firewall rule by UUID. Does not take effect until opnsense_apply_firewall_changes is called.",
        |s, a| async move { s.opnsense_update_firewall_rule(Parameters(a)).await },
    );
    let builder = register::<FirewallRuleUuidArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_delete_firewall_rule",
        "Delete an OPNsense firewall rule by UUID. Does not take effect until opnsense_apply_firewall_changes is called.",
        |s, a| async move { s.opnsense_delete_firewall_rule(Parameters(a)).await },
    );
    let builder = register::<ToggleFirewallRuleArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_toggle_firewall_rule",
        "Enable or disable an OPNsense firewall rule by UUID, without deleting it. Pass enabled to set it explicitly, or omit it to just flip the current state. Does not take effect until opnsense_apply_firewall_changes is called.",
        |s, a| async move { s.opnsense_toggle_firewall_rule(Parameters(a)).await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_apply_firewall_changes",
        "Apply pending OPNsense firewall changes (reloads the live ruleset). Call this after opnsense_create_firewall_rule, opnsense_update_firewall_rule, opnsense_delete_firewall_rule, or opnsense_toggle_firewall_rule -- none of those take effect on their own.",
        |s, _a| async move { s.opnsense_apply_firewall_changes().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_dhcp_leases",
        "List every current DHCP lease on OPNsense: IP address, MAC address, hostname, and lease state. Answers \"what's on my network right now\".",
        |s, _a| async move { s.opnsense_list_dhcp_leases().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_vlans",
        "List every VLAN interface currently configured on OPNsense, with its UUID, parent interface, and tag. Call this to find a VLAN's UUID before getting, updating, or deleting it.",
        |s, _a| async move { s.opnsense_list_vlans().await },
    );
    let builder = register::<VlanUuidArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_get_vlan",
        "Get one OPNsense VLAN's full field set by UUID. Call opnsense_list_vlans first to find UUIDs.",
        |s, a| async move { s.opnsense_get_vlan(Parameters(a)).await },
    );
    let builder = register::<CreateVlanArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_create_vlan",
        "Create a new VLAN interface on OPNsense. Does not take effect until opnsense_apply_vlan_changes is called.",
        |s, a| async move { s.opnsense_create_vlan(Parameters(a)).await },
    );
    let builder = register::<UpdateVlanArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_update_vlan",
        "Update an existing OPNsense VLAN by UUID. Does not take effect until opnsense_apply_vlan_changes is called.",
        |s, a| async move { s.opnsense_update_vlan(Parameters(a)).await },
    );
    let builder = register::<VlanUuidArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_delete_vlan",
        "Delete an OPNsense VLAN by UUID. Does not take effect until opnsense_apply_vlan_changes is called.",
        |s, a| async move { s.opnsense_delete_vlan(Parameters(a)).await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_apply_vlan_changes",
        "Apply pending OPNsense VLAN changes. Call this after opnsense_create_vlan, opnsense_update_vlan, or opnsense_delete_vlan -- none of those take effect on their own.",
        |s, _a| async move { s.opnsense_apply_vlan_changes().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_arp_entries",
        "List OPNsense's current ARP table: IP address, MAC address, hostname (if known), and interface for every neighbor it has resolved.",
        |s, _a| async move { s.opnsense_list_arp_entries().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_routes",
        "List OPNsense's system routing table (the same information `netstat -rn` shows on the firewall itself).",
        |s, _a| async move { s.opnsense_list_routes().await },
    );
    let builder = register::<NoArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_backup_providers",
        "List every configured OPNsense backup provider. Every install has at least `this` (the firewall's own local config history); more appear if a remote provider (Nextcloud, Google Drive, ...) is configured.",
        |s, _a| async move { s.opnsense_list_backup_providers().await },
    );
    let builder = register::<BackupHostArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_list_backups",
        "List every backup available from one OPNsense backup provider (host). Call opnsense_list_backup_providers first to find provider ids.",
        |s, a| async move { s.opnsense_list_backups(Parameters(a)).await },
    );
    let builder = register::<DownloadBackupArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_download_backup",
        "Download one OPNsense backup's raw config.xml content, or the current running config if no backup is named. Call opnsense_list_backups first to find backup names.",
        |s, a| async move { s.opnsense_download_backup(Parameters(a)).await },
    );
    register::<RestoreBackupArgs, _, _, _>(
        builder,
        server,
        rt,
        "opnsense_restore_backup",
        "Revert OPNsense's running configuration to a previous backup. Takes effect immediately -- OPNsense reloads its configuration as part of this call, there's no separate apply step. Call opnsense_list_backups first to find backup names.",
        |s, a| async move { s.opnsense_restore_backup(Parameters(a)).await },
    )
}

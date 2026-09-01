//! OPNsense tools: system status, service control, interfaces, firewall
//! aliases, and gateways.

use rmcp::{Json, handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use rusty_mcp::ToolError;
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

#[tool_router(router = opnsense_tools, vis = "pub(crate)")]
impl HomelabServer {
    /// Firmware version, running kernel, pending updates, service health.
    #[tool(
        description = "Get the OPNsense firewall's overall system status: firmware version, running kernel, pending updates, and per-service health."
    )]
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
    #[tool(
        description = "List every service OPNsense's service supervisor knows about, with its running state and short id (used by opnsense_service_control)."
    )]
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
    #[tool(
        description = "Start, stop, or restart a named OPNsense service. Call opnsense_list_services first to find valid service ids."
    )]
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
    #[tool(
        description = "List every network interface OPNsense knows about, keyed by device name."
    )]
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
    #[tool(
        description = "List every firewall alias currently configured on OPNsense (name, type, and contents)."
    )]
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
    #[tool(
        description = "List every configured gateway on OPNsense with its monitor status (none/loss/down) and ping latency."
    )]
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
    #[tool(
        description = "List every firewall rule currently configured on OPNsense, with its UUID, enabled state, action, interface, direction, protocol, and source/destination. Call this to find a rule's UUID before getting, updating, deleting, or toggling it."
    )]
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
    #[tool(
        description = "Get one OPNsense firewall rule's full field set by UUID. Call opnsense_list_firewall_rules first to find UUIDs."
    )]
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
    #[tool(
        description = "Create a new OPNsense firewall rule. Does not take effect until opnsense_apply_firewall_changes is called."
    )]
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
    #[tool(
        description = "Update an existing OPNsense firewall rule by UUID. Does not take effect until opnsense_apply_firewall_changes is called."
    )]
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
    #[tool(
        description = "Delete an OPNsense firewall rule by UUID. Does not take effect until opnsense_apply_firewall_changes is called."
    )]
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
    #[tool(
        description = "Enable or disable an OPNsense firewall rule by UUID, without deleting it. Pass enabled to set it explicitly, or omit it to just flip the current state. Does not take effect until opnsense_apply_firewall_changes is called."
    )]
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
    #[tool(
        description = "Apply pending OPNsense firewall changes (reloads the live ruleset). Call this after opnsense_create_firewall_rule, opnsense_update_firewall_rule, opnsense_delete_firewall_rule, or opnsense_toggle_firewall_rule -- none of those take effect on their own."
    )]
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
    #[tool(
        description = "List every current DHCP lease on OPNsense: IP address, MAC address, hostname, and lease state. Answers \"what's on my network right now\"."
    )]
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
    #[tool(
        description = "List every VLAN interface currently configured on OPNsense, with its UUID, parent interface, and tag. Call this to find a VLAN's UUID before getting, updating, or deleting it."
    )]
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
    #[tool(
        description = "Get one OPNsense VLAN's full field set by UUID. Call opnsense_list_vlans first to find UUIDs."
    )]
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
    #[tool(
        description = "Create a new VLAN interface on OPNsense. Does not take effect until opnsense_apply_vlan_changes is called."
    )]
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
    #[tool(
        description = "Update an existing OPNsense VLAN by UUID. Does not take effect until opnsense_apply_vlan_changes is called."
    )]
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
    #[tool(
        description = "Delete an OPNsense VLAN by UUID. Does not take effect until opnsense_apply_vlan_changes is called."
    )]
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
    #[tool(
        description = "Apply pending OPNsense VLAN changes. Call this after opnsense_create_vlan, opnsense_update_vlan, or opnsense_delete_vlan -- none of those take effect on their own."
    )]
    pub async fn opnsense_apply_vlan_changes(&self) -> Result<Json<JsonResult>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .apply_vlan_changes()
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

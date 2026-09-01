# rusty_opnsense

An async client for the [OPNsense](https://opnsense.org/) REST API: system
status, service listing/control, interface listing, firewall alias export,
and gateway status. Built on [`rusty_request`](../rusty_request), this
workspace's own async HTTP client, rather than `reqwest`.

Used by [`rusty_homelab_mcp`](../rusty_homelab_mcp) to expose OPNsense
control as MCP tools, but has no dependency on MCP/`rmcp` itself -- reusable
from a CLI, a different automation tool, or a test harness.

## Example

```rust,no_run
use rusty_opnsense::{OpnsenseClient, OpnsenseConfig, ServiceAction};

# async fn run() -> rusty_opnsense::Result<()> {
let client = OpnsenseClient::new(OpnsenseConfig {
    base_url: "https://opnsense.lan".to_string(),
    key: std::env::var("OPNSENSE_KEY").unwrap(),
    secret: std::env::var("OPNSENSE_SECRET").unwrap(),
    // OPNsense ships a self-signed certificate by default.
    insecure: true,
    timeout: None,
});

let status = client.system_status().await?;
client.service_control("unbound", ServiceAction::Restart).await?;
# Ok(())
# }
```

## Authentication

API key/secret only, sent as HTTP Basic auth (key as username, secret as
password) -- generated per user under **System -> Access -> Users**, "API
keys" section, independent of that user's own login password.

## Scope

Every method returns OPNsense's own JSON (`serde_json::Value`) as-is, rather
than a hand-maintained struct per endpoint: unlike Proxmox's uniform
`{"data": ...}` envelope, OPNsense's response shape varies by endpoint (a
plain object here, `{"rows": [...], "rowCount": N, ...}` there) and by
installed plugin version. See the
[API module reference](https://docs.opnsense.org/development/api.html) for
what each endpoint returns.

Covered: system status, service search/start/stop/restart, interface
listing, firewall alias export, gateway status, firewall rule CRUD
(list/get/create/update/delete/toggle) plus applying pending rule changes,
DHCP lease listing, and VLAN CRUD (list/get/create/update/delete) plus
applying pending VLAN changes. Not covered (open to a PR): general
interface settings writes (enable/blockpriv/blockbogons on a physical
interface) -- no stable, documented core REST endpoint for this was found,
as opposed to guessing one the way the OPNSenseMCP reference implementation
does; VPN (WireGuard/OpenVPN/IPsec) status; backup/config management.

Firewall rule and VLAN writes (`create_firewall_rule`/`update_firewall_rule`/
`create_vlan`/`update_vlan`) take their field set as a passthrough
`serde_json::Value` rather than a typed struct, for the same reason every
other method returns raw JSON: the valid field set depends on context (a
rule's own `ipprotocol`/`protocol`, in the firewall case), not one fixed
schema. None of the writes in either config area take effect until that
area's own `apply_firewall_changes`/`apply_vlan_changes` is called --
OPNsense buffers changes per config area the same way the web UI's "Apply
changes" button implies.

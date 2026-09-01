# rusty_proxmox

An async client for the [Proxmox VE](https://www.proxmox.com/en/proxmox-virtual-environment)
REST API: cluster/node listing, guest (QEMU VM and LXC container) listing and
status, and guest power control. Built on
[`rusty_request`](../rusty_request), this workspace's own async HTTP client,
rather than `reqwest`.

Used by [`rusty_homelab_mcp`](../rusty_homelab_mcp) to expose Proxmox control
as MCP tools, but has no dependency on MCP/`rmcp` itself -- reusable from a
CLI, a different automation tool, or a test harness.

## Example

```rust,no_run
use rusty_proxmox::{GuestKind, PowerAction, ProxmoxClient, ProxmoxConfig};

# async fn run() -> rusty_proxmox::Result<()> {
let client = ProxmoxClient::new(ProxmoxConfig {
    base_url: "https://pve.lan:8006".to_string(),
    token_id: "automation@pve!homelab-mcp".to_string(),
    token_secret: std::env::var("PROXMOX_TOKEN_SECRET").unwrap(),
    // Proxmox ships a self-signed certificate by default.
    insecure: true,
    timeout: None,
});

let nodes = client.list_nodes().await?;
let vms = client.list_guests("pve", GuestKind::Qemu).await?;
client
    .guest_power("pve", GuestKind::Qemu, 100, PowerAction::Start)
    .await?;
# Ok(())
# }
```

## Authentication

API token auth only (`PVEAPIToken=<user>@<realm>!<token-id>=<secret>`) --
the form Proxmox documents for automation, as opposed to the ticket/CSRF-token
pair the web UI itself uses. Create a token under **Datacenter -> Permissions
-> API Tokens**; the secret is shown once, at creation time.

## Scope

Every method returns Proxmox's own JSON (`serde_json::Value`) rather than a
hand-maintained struct per endpoint -- the response shape varies with guest
configuration and storage/network setup, and is already documented by the API
viewer bundled with every Proxmox install
(`https://<host>:8006/pve-docs/api-viewer/`). `guest_power` is the one
exception: it unwraps the task ID (`UPID:...`) Proxmox hands back for the
asynchronous action.

Covered: node listing and status, guest listing and status, guest power
actions (start/stop/shutdown/reboot/suspend/resume). Not covered (open to a
PR): task status polling, storage/backup management, cluster/HA
configuration, guest creation/cloning.

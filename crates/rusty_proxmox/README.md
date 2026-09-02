# rusty_proxmox

An async client for the [Proxmox VE](https://www.proxmox.com/en/proxmox-virtual-environment)
REST API: cluster/node listing, guest (QEMU VM and LXC container) listing,
status, power control, config, lifecycle (create/delete/clone/migrate),
snapshots, cluster resources, storage, and backups. Built on
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
let upid = client
    .guest_power("pve", GuestKind::Qemu, 100, PowerAction::Start)
    .await?;
// Power actions run asynchronously -- poll the task to know when it's done.
let status = client.task_status("pve", &upid).await?;
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
(`https://<host>:8006/pve-docs/api-viewer/`). Every asynchronous action
(`guest_power`, `create_guest`, `delete_guest`, `clone_guest`,
`migrate_guest`, `create_snapshot`, `delete_snapshot`, `rollback_snapshot`,
`run_backup`) is the exception: it unwraps the task ID (`UPID:...`) Proxmox
hands back instead of waiting for the action to finish -- poll it with
`task_status`/`task_log`.
`create_guest`/`update_guest_config`/`clone_guest`/`migrate_guest`/
`create_snapshot`/`run_backup` take their fields as a passthrough
`serde_json::Value` for the same reason: the valid field set differs between
QEMU and LXC (or, for `run_backup`, by mode/target) and by what's already
configured, not one fixed schema.

Covered: node listing and status, guest listing/status/config, guest power
actions (start/stop/shutdown/reboot/suspend/resume), guest create/delete/
clone/migrate, snapshot list/create/delete/rollback, cluster resources
overview, storage listing (datacenter- and node-level), backup job listing
and on-demand runs, and task status/log polling for every asynchronous
action above. Every endpoint was verified directly against Proxmox's own API
schema (`apidata.js`, the file backing the interactive API viewer) rather
than assumed. Not covered (open to a PR): HA/SDN/Ceph configuration, access
control, resource pools.

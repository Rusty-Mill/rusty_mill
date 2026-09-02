# rusty_homelab_mcp

An MCP server for controlling a homelab: [Proxmox VE](https://www.proxmox.com/en/proxmox-virtual-environment)
and [OPNsense](https://opnsense.org/) today, more backends welcome. Built on
[`rusty-mcp`](../rusty_mcp/crates/rusty-mcp), this workspace's own scaffold
for MCP servers, targeting spec 2026-07-28.

The Proxmox and OPNsense API clients themselves live in their own reusable
crates, [`rusty_proxmox`](../rusty_proxmox) and [`rusty_opnsense`](../rusty_opnsense)
-- this crate is the thin MCP layer on top: argument schemas, tool
descriptions, and wiring, nothing HTTP-specific.

## Running it

Both backends are optional and independent -- configure whichever ones are
reachable from wherever this server runs. Every tool is always listed
regardless of what's configured; calling one against an unconfigured backend
returns a clear error naming the flags to set, rather than the tool silently
vanishing from the list.

Over stdio, what a desktop client launches:

```bash
cargo run -p rusty_homelab_mcp -- \
    --proxmox-url https://pve.lan:8006 \
    --proxmox-token-id automation@pve!homelab-mcp \
    --proxmox-token-secret xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx \
    --proxmox-insecure \
    --opnsense-url https://opnsense.lan \
    --opnsense-key xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \
    --opnsense-secret yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy \
    --opnsense-insecure
```

Over Streamable HTTP: add `--transport http --bind 127.0.0.1:8080`.

Every flag has an environment fallback (`PROXMOX_URL`, `PROXMOX_TOKEN_ID`,
`PROXMOX_TOKEN_SECRET`, `PROXMOX_INSECURE`, `OPNSENSE_URL`, `OPNSENSE_KEY`,
`OPNSENSE_SECRET`, `OPNSENSE_INSECURE`, plus the scaffold's own
`MCP_TRANSPORT`/`MCP_BIND`/...) -- `--help` lists the rest.

`--proxmox-insecure`/`--opnsense-insecure` skip TLS certificate verification.
Both Proxmox and OPNsense ship a self-signed certificate by default, which
most homelabs never replace; never set these for a host reachable outside a
trusted network.

### Getting credentials

- **Proxmox**: Datacenter -> Permissions -> API Tokens. The secret is shown
  once, at creation time. `--proxmox-token-id` is the full
  `<user>@<realm>!<token-id>` form, e.g. `automation@pve!homelab-mcp`.
- **OPNsense**: System -> Access -> Users -> (your user) -> API keys. The
  secret is shown once, at creation time.

## Wiring it into a client

```json
{
  "mcpServers": {
    "rusty_homelab_mcp": {
      "command": "/absolute/path/to/target/release/rusty_homelab_mcp",
      "env": {
        "PROXMOX_URL": "https://pve.lan:8006",
        "PROXMOX_TOKEN_ID": "automation@pve!homelab-mcp",
        "PROXMOX_TOKEN_SECRET": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
        "PROXMOX_INSECURE": "true"
      }
    }
  }
}
```

## Tools

**Proxmox** (`rusty_proxmox`): `proxmox_list_nodes`, `proxmox_node_status`,
`proxmox_list_guests`, `proxmox_guest_status`, `proxmox_guest_power` (start/
stop/shutdown/reboot/suspend/resume a QEMU VM or LXC container),
`proxmox_task_status`/`proxmox_task_log` (poll the UPID `proxmox_guest_power`
and other asynchronous actions return), `proxmox_guest_config`/
`proxmox_update_guest_config`, `proxmox_create_guest`/`proxmox_delete_guest`/
`proxmox_clone_guest`/`proxmox_migrate_guest`, `proxmox_list_snapshots`/
`proxmox_create_snapshot`/`proxmox_delete_snapshot`/
`proxmox_rollback_snapshot`, `proxmox_cluster_resources`,
`proxmox_list_storage`/`proxmox_node_storage_status`,
`proxmox_list_backup_jobs`/`proxmox_run_backup`.

**OPNsense** (`rusty_opnsense`): `opnsense_system_status`,
`opnsense_list_services`, `opnsense_service_control` (start/stop/restart),
`opnsense_list_interfaces`, `opnsense_list_firewall_aliases`,
`opnsense_list_gateways`, `opnsense_list_firewall_rules`/
`opnsense_get_firewall_rule`/`opnsense_create_firewall_rule`/
`opnsense_update_firewall_rule`/`opnsense_delete_firewall_rule`/
`opnsense_toggle_firewall_rule` (firewall rule CRUD -- none of these take
effect until `opnsense_apply_firewall_changes` is called),
`opnsense_list_dhcp_leases`, `opnsense_list_vlans`/`opnsense_get_vlan`/
`opnsense_create_vlan`/`opnsense_update_vlan`/`opnsense_delete_vlan` (VLAN
CRUD -- none of these take effect until `opnsense_apply_vlan_changes` is
called), `opnsense_list_arp_entries`, `opnsense_list_routes`,
`opnsense_list_backup_providers`/`opnsense_list_backups`/
`opnsense_download_backup`/`opnsense_restore_backup`.

Most tools return the backend's own JSON as structured content under a
`result` field (`{"result": ...}`), unopinionated about the shape of `result`
itself (see each client crate's README for why). The wrapper exists because
MCP requires structured tool output to be a JSON object at the top level, and
several endpoints (e.g. `proxmox_list_nodes`, `opnsense_list_services`) return
a bare JSON array. `proxmox_guest_power`/`proxmox_create_guest`/
`proxmox_clone_guest`/... and `opnsense_download_backup` return plain text
instead: the Proxmox tools return a task ID (a `UPID:...` string) since
Proxmox runs those actions asynchronously rather than waiting for them to
finish (pass that UPID to `proxmox_task_status`/`proxmox_task_log` to find
out when it's done), and `opnsense_download_backup` returns OPNsense's own
raw `config.xml`, which isn't JSON at all.

## Adding a backend

1. Add (or reuse) a client crate for it, shaped like `rusty_proxmox`/
   `rusty_opnsense`: a small async client returning the backend's own JSON,
   with no dependency on MCP/`rmcp`.
2. Add a `src/tools/<backend>.rs` module: argument structs, a
   `#[tool_router(router = <backend>_tools, vis = "pub(crate)")]` block on
   `impl HomelabServer`, one method per tool.
3. Register the module in `src/tools/mod.rs`, add the client as a field on
   [`HomelabServer`](src/server.rs) alongside a `pub(crate) fn <backend>(&self)`
   accessor (mirrors `proxmox`/`opnsense`), and add `+ Self::<backend>_tools()`
   in `HomelabServer::new`.
4. Add the backend's URL/credential flags to [`Cli`](src/config.rs) and a
   `<backend>_config()` builder, following `proxmox_config`/`opnsense_config`.

## Testing

```bash
cargo test -p rusty_homelab_mcp
```

`tests/tools.rs` connects a real client over an in-memory pipe -- the same
code path the stdio transport takes -- against `HomelabServer` instances
backed by either real (but unconfigured) or mock-HTTP-backed clients, so it
covers dispatch, schema generation, serialization, and the unconfigured-
backend error path, not just the tool functions in isolation.

See the [`rusty-mcp` README](../rusty_mcp/README.md) for what else the
scaffold gives you (resources, prompts, completions, authorization, tasks,
tracing/metrics, load shedding) that isn't wired up here yet.

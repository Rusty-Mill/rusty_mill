# rusty_fedora

An async client for [`rusty_fedora_agent`](../rusty_fedora_agent)'s local
HTTP API -- the unprivileged agent that runs on a Fedora Server host (e.g.
baileyai) and exposes scoped systemd/dnf/config-file control.

Same shape as [`rusty_opnsense`](../rusty_opnsense)/
[`rusty_proxmox`](../rusty_proxmox): built on
[`rusty_request`](../rusty_request), returns the agent's own JSON
(`serde_json::Value`) rather than a hand-maintained struct per endpoint,
and is consumed by [`rusty_homelab_mcp`](../rusty_homelab_mcp)'s `fedora`
module the same way those two are consumed by its `opnsense`/`proxmox`
modules.

## Example

```rust,no_run
use rusty_fedora::{FedoraAgentClient, FedoraAgentConfig, ServiceAction};

# async fn run() -> rusty_fedora::Result<()> {
let client = FedoraAgentClient::new(FedoraAgentConfig {
    base_url: "http://100.x.y.z:8765".to_string(), // a private/Tailscale address
    timeout: None,
});

let status = client.system_status().await?;
client.service_control("ollama.service", ServiceAction::Restart).await?;
# Ok(())
# }
```

See `rusty_fedora_agent/README.md` for the full endpoint list and
response shapes.

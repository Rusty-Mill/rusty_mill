---
category: Changed
changelog: `rusty_homelab_mcp`, `rusty_fedora`, `rusty_opnsense` and `rusty_proxmox` run on `rusty_tokio` instead of tokio; `rusty_tokio` gains tokio's `Handle::block_on` (#905).
---
## 2026-10-24 - homelab on rusty_tokio (#905)

- **Changed:** `rusty_homelab_mcp`, `rusty_fedora`, `rusty_opnsense` and `rusty_proxmox` depend on `rusty_tokio` instead of tokio, and none keeps a normal or dev tokio dependency.
  - **Crate rename:** `tokio::runtime::Handle`, `task::spawn_blocking`, `#[tokio::main]` and `#[tokio::test]` become their `rusty_tokio` paths.
  - **Test attributes:** the `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` attributes become `#[rusty_tokio::test(worker_threads = 2)]`, which is the same runtime, since `rusty_tokio`'s test attribute is multi-threaded by default.
  - **Libraries:** the three libraries drop `rusty_request`'s `tokio` feature. They needed it only because their one consumer, `rusty_homelab_mcp`, ran on tokio.
  - **Public API:** `rusty_homelab_mcp`'s `wire_server` and tool registration take a `rusty_tokio::runtime::Handle`. No other crate depends on it.
- **Added (`rusty_tokio`, tokio parity):** `Handle::block_on`, which runs a future to completion on the calling thread with that runtime ambient. The MCP server's blocking handlers use it to drive async tool calls from the transport's own threads. On a current-thread runtime it drives the runtime's queue itself, as `Runtime::block_on` does.
- **Verified:**
  - The four crates' tests pass with all features: 134, the same count as on tokio. A new `rusty_tokio` test runs `Handle::block_on` from an outside thread on both runtime flavors, with a spawn and a sleep.
  - The binary itself, over stdio, answers MCP `initialize`, `tools/list` (58 tools) and a `tools/call`.
  - clippy (`-D warnings`), fmt and the Windows cross-check pass.

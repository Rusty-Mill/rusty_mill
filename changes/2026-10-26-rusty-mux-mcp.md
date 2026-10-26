---
category: Added
changelog: rusty_mux MCP server: `rusty-mux-mcp` serves the request catalog as MCP tools over stdio, each call run as `rusty-mux-ctl` runs it, so agents can create blocks, send input and read screens and status.
---
## 2026-10-26 - rusty_mux MCP server

- **Added:** `rusty_mux_mcp` with the `rusty-mux-mcp [--socket PATH] [--timeout SECS]` binary on `rusty_mcp_server` (stdio). Each catalog method is a tool with the same name, summary and argument schema that `rusty-mux-ctl api describe` prints, so the CLI and the tools come from one catalog.
- **How a call runs:** each call runs as the equivalent `rusty-mux-ctl` command line on a connection of its own, so ids take names, positions, id suffixes and paths, and a missing id defaults from `$RUSTY_MUX_*`. A call returns ctl's plain output, trailing spaces and blank lines kept and only its final newline dropped, or `ok` when ctl prints nothing. A refusal or an unreachable server comes back as a tool error with the reason. A call gives up on a silent server at its deadline (30 s by default) or as soon as the client cancels it, closing its connection.
- **Which methods are tools:** attach, detach, acquire-lease, release-lease and sync mean nothing on a connection that closes after one call, so they are not tools. `input` and `resize` attach and take the lease first in the same call, so they fail while someone else holds it. Tools carry hints: reads are read-only and closing is destructive. `input` is destructive and open-world, since what it types can delete data or reach any system.
- **Changed:** `rusty_mux_ctl` exposes `input_schema`, `Env::from_process`, and `run_until` with a `Stop` (a deadline and a cancellation check that bound its socket I/O) for the MCP server. A malformed `$RUSTY_MUX_*` id warning now starts with `rusty-mux:`.
- **Known limitations:** stdio only, with no HTTP transport, and no `events` tool yet. Requests in one MCP session run concurrently, so an agent that needs one call to finish before the next must wait for its reply. An id parameter is still listed as required even when the environment would supply it.

# Roadmap

Phases from the design brief §9. Each exit criterion is a test or a measured number. Phase N+1 starts only after the owner accepts phase N's go/no-go.

## Phase 0: engine prerequisites (done 2026-10-23, two targets open)

Status: items 1, 2 and 4 landed (#872, #894, #939); item 3 landed as #930 plus #936. Open: the MB/s floors (#889) and reliable ≤ 11.2 ms encode (#939). See CAPABILITIES.

1. #872 lands: instruction-count gate in `required-gate`.
2. #855 lands: per-code-point widths by default, mode 2027 opt-in; `vim_edit` matches the oracle.
3. Ring-buffer screen + recycled scrollback rows (backlog #1 + #2): `scroll_heavy` ≥ 16 MB/s, `ascii_throughput` ≥ 58 MB/s, memmove < 10% of instructions (callgrind); fidelity gate green; memory per 10k lines and peak RSS reported before and after.
4. Snapshot serializer (backlog #16): round-trip decode equals the source grid under the fidelity dump; 200x50 + 10k lines encodes in ≤ 11.2 ms and ≤ 7.8 MB.

Engine tuning stops here. Backlog #3 (GUI frame path) and #4 (printable fast path) wait unless a mux benchmark shows they are the bottleneck.

## Phase 1: vertical slice (Linux; Windows once spike 1 answers)

Runtime: `rusty_tokio` (DECISIONS), extended where needed; no tokio in any mux binary.

Scope: `core` (< 3k LOC, zero I/O, IDs via `rusty_uuid`), `proto` (`rusty_wire` + LEB128 varint; u32 BE length + u8 type + tagged fields; skip unknown ids; feature bits; < 4k LOC), `server` (PTYs via `platform::pty`, one `rusty_term_core` per PTY, snapshot then ready then raw byte tee, single-writer input, per-client viewport), and a throwaway TUI client over a Unix socket. No auth, TLS, roaming, parking, MCP or scripting.

Exit criteria:
- Two clients attach to one PTY and converge to the same grid (integration test).
- A client detaches and reattaches via snapshot; its grid equals the server's (integration test).
- An output flood does not starve input: a keystroke round-trips within a bound while the PTY floods (integration test; bound set in the PR).
- Fidelity gate and benchmark gate green.

## Phase 2: API and automation

Schema-first API, CLI, resumable events, block methods, program status, MCP. Exit: CLI and MCP are generated from one schema catalog (test); CLI exit codes 0/1/2/3/4/130 covered by tests; events resume after a disconnect without loss or duplication (test); status OSC round-trips server to client (test).

Started 2026-10-25 (owner go-ahead). One PR per step, in order:
1. Request schema catalog in `rusty_mux_proto::schema` (C17). Done (#1051).
2. Resumable event stream: sequence numbers and replay (C05). In progress.
3. `rusty_mux_ctl`, generated from the catalog, with `--json` and exit codes (C04).
4. Block methods and screen capture (C06, C18).
5. Program status OSC, `rusty_mux_status` (C20).
6. `rusty_mux_mcp` on `rusty_mcp_server`, generated from the same catalog (C07).
7. Scripting (C19), last.

## Phase 3: identity and remote

Identity, login server mode, roaming transport, sharing. Exit: a remote session survives a client IP change (test, after spike 4); login mode maps peer credentials or Tailscale identity to an account (test); a shared session enforces input leases (test).

## Phase 4: clients and Windows

Web and native clients, ConPTY server and client. Exit: the Phase 1 acceptance suite passes on Windows; web and native clients pass the convergence test.

## Phase 5: scale and hardening

Parking, encryption at rest, audit. Exit: idle-terminal memory measured with and without parking (spike 5 numbers, ours); parked snapshots are encrypted and round-trip to the same grid (test); an outbound-connection audit of the release finds no egress.

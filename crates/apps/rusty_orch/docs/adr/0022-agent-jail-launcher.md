# ADR-0022: `lel-jail`, the launcher a mux agent block runs

- **Status:** Accepted (Let Em Loose D82, design approved 2026-11-05; build defaults D85)
- **Date:** 2026-11-05

## Context

Agents get mux-hosted ptys (D81). The mux forks the block's program on a
pty it owns, so that program is the first process orch's jail controls.
It must put the agent in the session's cgroup, keep only the channels the
design allows (the pty, the slot lock on fd 3, ACP on fd 4) and confine it
before the agent's code runs (mux-hosted-agents-design.md §2 step 5, §4).

## Decision

- A new crate, `orch-jail`: the `jail.json` type (`JailFile` on the wire,
  `JailSpec` once validated) and the `lel-jail` binary. orch writes the file
  and the mux runs `lel-jail <jail.json>`, so writer and reader share one
  type, and orch-bbp gains no Linux dependency.
- The order is fixed: validate; require fds 3 and 4 (fail closed); join the
  cgroup; close every fd above 4; read the token by reference; then hand off
  to rusty_sandbox's helper with `Sockets::InternetOnly`, the profile the
  bbp_host harness already uses. A setup failure writes the status file,
  prints to the pty and exits 125. Nothing execs unconfined.
- The file can't weaken the jail: there is no sockets field, `RUSTY_MUX_*`
  variables are refused (D13), and the token file must lie outside every
  root the agent can reach.

## Consequences

- v0 has no namespaces, overlay or gateway-only egress. `InternetOnly`
  allows any IPv4/IPv6 egress, short of 600-builder-session-design §4.3.
  Those #726 layers come later behind the same file (D82 item 7).
- The mux socket is kept out by the environment, the fd rule, seccomp and
  Landlock, not yet by a mount namespace, so the "socket path doesn't
  exist" test waits for that layer.
- Which credential the Claude CLI uses inside the jail is open (LEL Q15);
  the `token` slot carries only an orch-minted token until it's answered.

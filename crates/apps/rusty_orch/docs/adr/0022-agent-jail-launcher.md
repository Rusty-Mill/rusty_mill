# ADR-0022: `lel-jail`, the launcher a mux agent block runs

- **Status:** Accepted (Let Em Loose D104, design approved 2026-11-05; build defaults D107)
- **Date:** 2026-11-05

## Context

Agents get mux-hosted ptys (D103). The mux forks the block's program on a
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
  cgroup; close every fd above 4; resolve every root, the cwd and the token
  and check the token again on the resolved paths; read the token by
  reference; then hand the resolved roots, the ones that were checked, off
  to rusty_sandbox's helper with `Sockets::InternetOnly`, the profile the
  bbp_host harness already uses. A setup failure writes the status file,
  prints to the pty and exits 125. Nothing execs unconfined.
- The file can't weaken the jail: there is no sockets field, `RUSTY_MUX_*`
  variables are refused (D13), and the token file must lie outside every
  root the agent can reach. That is checked twice: on the paths as written
  (no `..` anywhere), then on what they resolve to, where neither the
  token nor its directory may be in or below a root.
- The second check is made on pinned objects. The launcher opens every
  root, the cwd and the token once as an `O_PATH` descriptor, judges each
  by its own `/proc/self/fd/N` link, and gives the helper those links in
  place of the names. Landlock's rules then land on exactly the objects
  that were checked, and the token is read through its descriptor. A peer,
  confined or not, that renames a root or swaps a symlink in after the
  check redirects only the name, never a rule. Resolution fails closed.
  As a further check, every part of a resolved path must be owned by root
  or the launcher's user, every directory above it must be writable by no
  one else unless sticky, and the token file must have exactly one link.

## Consequences

- v0 has no namespaces, overlay or gateway-only egress. `InternetOnly`
  allows any IPv4/IPv6 egress, short of 600-builder-session-design §4.3.
  Those #726 layers come later behind the same file (D104 item 7).
- The mux socket is kept out by the environment, the fd rule, seccomp and
  Landlock, not yet by a mount namespace, so the "socket path doesn't
  exist" test waits for that layer.
- Pinning fixes which objects are granted, not where they sit later. A
  token directory moved into a granted root after the launch would be
  reachable, so the token directory's parent must not be writable by any
  agent; orch must keep it outside every write root it grants.
- Which credential the Claude CLI uses inside the jail is open (LEL Q15);
  the `token` slot carries only an orch-minted token until it's answered.

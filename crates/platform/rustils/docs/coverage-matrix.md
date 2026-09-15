# Coverage matrix

Tracks rustils' actual platform/domain coverage against
`rusty_foundation_akb`'s charter (three platforms — Windows, Linux,
macOS — and a 20+-domain capability graph), per issue #115. This is a
scope-gap backlog, not a correctness problem: everything listed
"Not started" is an explicit, tracked absence, not an oversight.

## Platforms

| Platform | Status | Notes |
|---|---|---|
| Linux | Done | `platform-linux`, full `Fs`/`Process`/`Events`/`Net`/`Security` coverage |
| Windows (NT) | Done | `platform-windows`, same trait coverage minus BSD-only slices |
| BSD (incl. macOS) | Partial | `platform-bsd` — `Net` + a `Security` slice only, by explicit scope choice (`platform-bsd/src/lib.rs`); no `Fs`/`Process`/`Events` backend |
| macOS as a first-class target | **Not started — accepted long-term gap** | No `platform-macos` crate. Not a near-term goal: revisit if/when a named consumer needs it, per §3's no-speculation rule; `platform-bsd`'s existing `Net` coverage already serves macOS's most-forced use case (rusty_tokio's kqueue reactor) without a dedicated crate |

## Domains

| Domain | Status | Notes |
|---|---|---|
| Filesystem (`Fs`) | Done | Linux, Windows |
| Process management (`Process`) | Done | Linux, Windows |
| Networking — TCP/Unix/UDP (`Net`) | Done | Linux, Windows, BSD |
| Signals (`Events`) | Done | Linux, Windows (installation/empty-slot only — see `docs/behavior/events.md`) |
| Security — CSPRNG, sandboxing (Landlock/seccomp), trust anchors (`Security`) | Done (Linux); Partial (Windows, BSD) | See `docs/behavior/security.md` |
| Security — `CredentialStore` | Gated | Forcing consumer (nexus) checked 2026-07-20, not live yet — see `docs/architecture.md`'s gated-surfaces table |
| Terminal, PTY, Tun, Windowing, Registry/Config | Gated | Each unparks only when its named forcing consumer arrives — see `docs/architecture.md`'s gated-surfaces table |
| Memory / mapping | Not started | No named forcing consumer yet |
| Threading primitives (beyond process-level) | Not started | No named forcing consumer yet; async execution (thread-pool/reactor concerns) is explicitly out of scope for Layer 2 — see `docs/architecture.md`'s Execution and concurrency model section |
| Graphics | Not started | No named forcing consumer yet |
| Observability / diagnostics | Not started | No named forcing consumer yet |
| Secrets lifecycle (beyond `CredentialStore` get/set) | Not started | Rotation, attributes, multi-collection — see `docs/behavior/security.md`'s Deliberately unspecified section |
| Policy evaluation | Not started | No named forcing consumer yet |
| Plugin / module lifecycle | Not started | No named forcing consumer yet |
| Package management | Not started | No named forcing consumer yet |

## How this list changes

Per §3 (the PAL never speculates), a domain moves from "Not started" to
active work only when a real, named consumer forces it — see
`docs/architecture.md`'s "Gated future surfaces" table for the mechanism.
This matrix is a status snapshot, not a roadmap commitment; update it
alongside `docs/architecture.md` whenever a gated surface unparks or a
new platform/domain gap is identified.

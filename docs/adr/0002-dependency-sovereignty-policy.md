# ADR-0002: Dependency sovereignty policy

Status: Accepted
Date: 2026-09-03

## Context

`rusty_mill`'s stated purpose leans toward a handrolled, dependency-minimizing
Rust ecosystem, and the README documents many from-scratch implementations
(regex, JSON, HTTP/2, UUID, SIMD, and more). At the same time, an external
review of the workspace (199 manifests, 183 workspace members) found that
"no external dependencies" is not an accurate description of the monorepo as
a whole: 114 manifests declare at least one direct external normal
dependency, the resolved lockfile carries over a thousand registry package
instances, and several components -- `rusty_tls`'s rustls default path,
`rusty_sqlite`'s bundled SQLite, SQLx-based database adapters, Tauri desktop
shells, `rmcp`-based MCP crates -- are deliberate, documented, external-backed
decisions rather than accidents.

That gap is not itself a problem. It becomes one when "sovereignty" is used
as an inventory label rather than a mechanically checked classification:
without an explicit rule, a genuinely zero-dependency core crate, a crate
mid-migration off an external dependency, and a crate that will always need
an external adapter (a GUI toolkit, a database driver) all get described the
same way. The same review also found a concrete instance of the failure mode
this ambiguity enables: three manifests (`crates/rusty_term/l13`,
`crates/rusty_font`, `crates/rusty_gpu`) depended on `rusty_lsp` and
`rusty_simd` via a pinned git URL even though both are workspace members
with their own `crates/<name>` directory -- letting the git copy and the
workspace copy silently diverge, contrary to Atlas's same-workspace-source
requirement (`ATLAS-RWC-0050`). Those three were corrected to plain path
dependencies alongside this ADR.

## Decision

Adopt three dependency tiers, and require every crate to be classifiable
into exactly one:

| Tier | Rule | Examples in this workspace |
| --- | --- | --- |
| **S -- Sovereign** | No external normal or build dependencies. Dev-only external dependencies are allowed as independent test oracles and must not ship. | `rusty_std`, `rusty_libc`, `rusty_wire`, `rusty_json`, `rusty_regx` |
| **T -- Transitional** | External dependencies are present but are meant to be replaced or narrowed; each one needs an owner, a rationale, a first-party seam consumers code against, and a tracked replacement/parity milestone. | `rusty_request`'s pluggable async backend, crates mid-migration off `thiserror`/`uuid`/`base64`/`url` per issues #119-#121 |
| **A -- Adapter/application** | External integration is allowed behind a first-party boundary and is not expected to ever reach zero dependencies; it must not be marketed as dependency-free. | `rusty_tls` (rustls default path), `rusty_sqlite` (bundled SQLite), SQLx-based database adapters, Tauri desktop shells, `rmcp`-based MCP crates |

A crate's tier is a claim about its own manifest, not a judgment on the
crates it depends on within the workspace -- a Tier A crate may depend on
Tier S crates without pulling them into Tier A.

"External" for this ADR means: any dependency resolved from crates.io or a
git remote, in `[dependencies]` or `[build-dependencies]` (`[dev-
dependencies]` are exempt when they do not ship, per the S-tier rule above).
It does not cover Rust `std`, OS APIs called directly (`libc`-free syscalls,
Win32 FFI), or a proc-macro workspace member. Whether the Rust standard
library itself counts as an "external dependency" for a stricter future
tier is explicitly out of scope for this ADR; today's tiers are defined
relative to the Cargo dependency graph, not `#![no_std]` status.

Regardless of tier, every same-workspace first-party crate must resolve via
a local path (or `{ workspace = true }` against a `[workspace.dependencies]`
entry that itself uses `path`), never via a git or registry source, per
`ATLAS-RWC-0050`. This is checked in CI (`.github/workflows/ci.yml`'s
`dependency-policy` job, backed by `.github/scripts/
check_workspace_deps.py`): it fails a PR that resolves any workspace
member's name from a git source anywhere in the graph, which is exactly the
`rusty_lsp`/`rusty_simd` failure mode this ADR was written in response to.

## Alternatives considered

**A single "no external dependencies" rule, enforced everywhere.** This
matches the README's aspirational language most literally, but it is not
what the workspace does today (TLS, SQL, desktop shells, and MCP tooling all
have deliberate external-backed designs recorded in their own ADRs), and
forcing it now would mean either rewriting cryptography, database engines,
and GUI/windowing stacks under time pressure -- exactly the kind of
handrolling-for-its-own-sake this workspace's own README warns is not the
goal for security-sensitive code -- or quietly ignoring the rule for entire
subsystems, which is the ambiguity this ADR exists to remove.

**No tiers; keep relying on narrative documentation (READMEs, per-crate
ADRs) to describe each crate's external-dependency posture.** This is
closer to the status quo the review assessed. It does not scale past a
handful of crates: with 183 workspace members and 162 unique external
dependency names, a reader cannot tell from the aggregate inventory alone
which external dependencies are permanent, which are scheduled for
replacement, and which indicate an unreviewed regression, and nothing
prevents a new manifest from adding an external dependency without anyone
deciding which tier it belongs to.

## Consequences

- Every crate's tier should be recorded where its own dependency decisions
  already live (its README or `docs/adr/`, per this ADR's own
  same-workspace-source and external-dependency definitions) rather than
  only in a root-level table that inevitably drifts as the workspace grows;
  a generated, CI-validated ledger cross-referencing manifests to tiers is
  tracked as follow-up work rather than introduced by this ADR.
- The `dependency-policy` CI job makes the `ATLAS-RWC-0050` same-workspace-
  source rule a merge-time check instead of a narrative claim: a future PR
  that reintroduces a git dependency shadowing a workspace member fails CI
  immediately instead of surfacing only in a future audit.
- This ADR does not itself resolve any Tier T crate to Tier S. Bounded
  replacement work (base64, URL, UUID, `libc`, narrow `thiserror` swaps)
  stays scoped to the issues already tracking it (#118-#121); high-cost
  replacements (async runtime, TLS/crypto, database engines, MCP SDKs,
  GUI/windowing/GPU stacks) are explicitly out of scope for any single PR
  and need their own forcing function and evidence program before being
  attempted.
- "Sovereign" is no longer a claim about the workspace as a whole. A crate,
  or an explicitly scoped shipping profile of one, can be described as
  sovereign; the workspace overall remains, and is expected to remain, a
  sovereign core surrounded by transitional and adapter-layer crates.

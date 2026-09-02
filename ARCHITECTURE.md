# Architecture

## Overview
`rusty_mill` is a Cargo workspace consolidating what used to be ~90+
independent `baileyrd/*`/`Rusty-Mill/*` repos into one repository, one
build, and one CI pipeline. Each crate was merged in via `git subtree`
under `crates/`, keeping its full original commit history. It is not a
single application — there is no one "system" with a request/response flow
running through it. It's a build/governance boundary: many independently-
purposed crates (a shell, a terminal emulator, a TLS stack, an async
runtime, protocol implementations, homelab clients, and more) that share a
lockfile, a CI pipeline, and a duplication/sovereignty review surface, not
a shared runtime or a shared domain model.

**Non-goal:** this is not a plan to merge these crates' *behavior* — each
stays its own crate with its own public API and its own reason to exist.
The monorepo's value is operational (one CI run, one place to find and fix
cross-crate duplication — see `my_loops/repo-inspector` in
`baileyrd/skill_pack`) and historical (git subtree preserves per-crate
history instead of squashing it away), not a step toward a single merged
codebase.

## Boundaries
Ports-and-adapters doesn't apply at the workspace level the way it would to
a single service — there's no one domain to keep free of I/O. It applies
*within* individual crates that have that shape (e.g. `rusty_search`'s
backend-agnostic `SearchBackend` trait with pluggable adapters per search
engine, or `rusty-db`'s driver abstraction). At the workspace level, the
closer analogue is a **dependency layering** convention, not ports/adapters:

| Layer | Crates (examples) | Notes |
| ---- | ---------- | ----- |
| Sovereign foundation | `rusty_std`, `rusty_libc`, `rusty_win32`, `rusty_wire`, `rusty_sync` | `no_std`/`alloc`, zero or near-zero external dependencies. Everything else in the workspace ultimately builds on this layer or on the real Rust `std`. |
| Protocol / format | `rusty_json`, `rusty_serde`, `rusty_http`, `rusty_h2`, `rusty_url`, `rusty_ansder`, `rusty_regx` | From-scratch implementations of a spec or wire format, largely dependency-free of each other except where one format is built on another (`rusty_ansder` on `rusty_wire`). |
| Runtime / platform | `rusty_tokio`, `rustils_async`, `platform-async*`, `threading` | Async runtime and OS-abstraction layer other crates opt into. |
| Application / client | `rusty_proxmox`, `rusty_opnsense`, `rusty_homelab_mcp`, `rusty_request`, `rusty_search-*` backends | Consume the layers below to talk to a specific external system. |
| Terminal / shell | `rusty_term`, `rush`, `mill-term`, `rusty_lines`, `rusty_ansi` | The interactive-tool cluster; several of these were the subject of the duplication sweeps below. |

This table is illustrative, not exhaustive — see each crate's own README
for what it actually depends on and provides; `Cargo.toml`'s `[workspace]
members` list is the authoritative membership list.

## Structure
Not a modular monolith in the generic `repo-config` default sense (that
default assumes one deployable service growing internal module
boundaries) — this is a **workspace of independently-versioned,
independently-purposed crates** sharing build/CI infrastructure. `ATLAS-300`
(`baileyrd/Atlas_Engineering_Standards_Library`, Rust Workspace and Cargo
Architecture) is now an active volume with a published workspace
requirement set (`ATLAS-RWC-*`: explicit membership, an explicit resolver,
shared metadata and dependency policy, workspace-local first-party
resolution, a committed lockfile), promoted from exercised evidence by
Atlas ADR-0006. This repo's structure predates that promotion and was not
derived from it; how the two line up, requirement by requirement, is
assessed in [`docs/atlas/`](./docs/atlas/) rather than restated here.
Feature-flag architecture is still deferred in ATLAS-300; that review
argues the trigger has fired on this repo's evidence (PRs #134 and #136).
This section therefore still describes the structure that exists here, not
one derived from the standard.

CI (`.github/workflows/ci.yml`) scopes each job to only the crates a PR
actually touches (plus transitive dependents) via an `affected_crates.py`
plan step, rather than always running `--workspace` across the full crate
count — the practical reason a monorepo this size stays fast to iterate on.

## Data flow
Not applicable at the workspace level for the reason given in Overview —
there's no single request/event flowing through the whole tree. See an
individual crate's own README/ARCHITECTURE (where it has one) for its own
data flow.

## Key decisions
See [docs/adr/](./docs/adr/) for decisions belonging to the workspace as a
whole (see ADR-0001 for the root series' remit, and how it relates to the
`docs/adr/` directories several individual crates already carry from
before the merge). ADR-0001 documents the consolidation decision itself.

Two duplication sweeps have already run against this workspace and are
worth knowing before proposing a new one (both by hand/an ad hoc session,
predating `repo-inspector`):
- [PR #10](https://github.com/Rusty-Mill/rusty_mill/pull/10) — 8 findings
  (issues #1–#8), 6 fixed (glob matching, SHA-1, `to_wide()`, `read_lines()`,
  Windows raw-mode flags, IFS splitting), 2 closed `no action` (Unix termios
  save/restore — same shape, deliberately different policy; a `no_std`
  rounding workaround in `rusty_font`). Issue #9 remains open (a capability
  gap in `rusty_regx::Glob`, not duplication).
- [PR #65](https://github.com/Rusty-Mill/rusty_mill/pull/65) — 2 more fixed
  (`rusty_rdp`'s byte cursor merged into `rusty_wire`; `rusty_ansder` split
  into itself plus a new `rusty_rag` crate), 3 more investigated and
  deferred as different-scoped tools sharing a name rather than true
  duplication (`rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER codec vs.
  `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs. `rusty_url::Url`).

## Non-goals
- Not a plan to collapse these crates into fewer, larger ones, or to give
  them a shared runtime/domain model — see Overview.
- Not a supply-chain/sovereignty audit by itself — `repo-inspector`'s
  sovereignty pass and `sovereignty-loop` cover that separately.
- Not a governance decision about *how* future crates get merged in (git
  subtree vs. otherwise) — that precedent is documented in ADR-0001, not
  re-litigated here.

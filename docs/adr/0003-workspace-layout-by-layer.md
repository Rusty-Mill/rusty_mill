# ADR-0003: Organize `crates/` by architectural layer

Status: Implemented
Date: 2026-09-15 (proposed) / 2026-09-16 (implementation complete)

Authorship: written by the Claude host under `/codex-build` on
2026-09-15 (the Codex builder could not run) and inspected by fresh,
separate Claude sessions; provenance details are in the companion review
`CODEX-MONOREPO-REVIEW-2026-09-15-organization.md`, "Limitations".
Independently reviewed by Codex once it could run (`PLAN-REVIEW-LOG.md`,
3 rounds, approved). Implemented across 6 merged PRs (#222-#227: Phase
0a metadata/checker/generated-map, 0b dependency hoisting, then the
1/platform, 2/foundation, 3/libs, 4/apps+tools moves), built by Codex and
independently inspected by the host each round — full build/inspection
record in `PLAN-REVIEW-LOG.md`. Every one of the 239 workspace members
now lives at the exact path this ADR's Appendix B specifies, verified
directly against the merged `main` after the final PR landed.

## Context

`rusty_mill` is 239 workspace members in 82 top-level directories under
`crates/` (root `Cargo.toml`, `[workspace] members`; counts from
`cargo metadata --no-deps --locked` at `cd789d573`). Every directory sits at
the same depth with no signal about what kind of thing it is. ADR-0001
deliberately kept each merged repo's directory intact, which was right for
the merge waves, but it leaves three problems a newcomer hits immediately:

1. **Reusable libraries live inside application-looking families.** The
   clearest case is the OS abstraction layer. `platform`, `platform-linux`,
   `platform-windows`, `platform-bsd`, `platform-mock`, `platform-parity`
   and `winargv` live under `crates/rustils/crates/` beside `coreutils`.
   `crates/rustils/README.md` does describe the family correctly ("a
   hand-rolled, Rust-native platform personality layer", line 3, with
   `coreutils` as the "reference consumer", line 37), but nothing outside
   that README says so: the directory is named like a coreutils port,
   `ARCHITECTURE.md`'s "Runtime / platform" row lists `rustils_async` and
   `platform-async*` but not `platform*` itself, and `platform` has 12
   dependents in other families (`rusty_tokio`, `rusty_tls`, `rusty_rdp`,
   `rusty_fedora_agent`, `sessionmgr-pty`, `nexus-rush`, `ts-magicsock`,
   `ts-tun`, the three `platform-async*` crates and `coreutils-async`)
   while `platform-linux` has 9. The layer everything else stands on is
   filed under a name that reads as one of its consumers.
2. **The layers exist but are not visible.** `ARCHITECTURE.md`'s
   "Boundaries" table names a sovereign foundation, a protocol/format
   layer, a runtime/platform layer and application layers, but says it is
   "illustrative, not exhaustive", nothing enforces it, and the tree does
   not reflect it. There are in fact three separate OS-abstraction families
   (`rustils`' `platform*`, `rustils_async`'s `platform-async*`/
   `reactor-core`/`threading`, and `rusty_test`'s `contract`/`compat`/
   `conformance`, whose README title is `portable-runtime-contract`), and
   no reader would find the third one under a directory called
   `rusty_test`.
3. **The map is hand-maintained.** `README.md`'s "How the crates relate"
   section (lines 336 to 1222 at this commit) narrates every dependency by
   hand. `docs/atlas/rusty-mill-atlas-evidence-review.md` (lesson 10)
   already records that this prose drifts.

The dependency graph itself is healthy: every path edge between members
already points from higher-level code to lower-level code under the layer
assignment below, with zero exceptions (see the appendices, which the
proof script in the review checks mechanically). The problem is purely one
of presentation and enforcement, which is why this ADR can propose a
layout that encodes the graph rather than one that has to change it.

## Decision

### Layer model

`crates/` gains five layer directories. Each existing family directory
moves under exactly one of them (or is split, per the rule below).

| Layer dir | Meaning | May depend on |
|---|---|---|
| `crates/foundation/` | ADR-0002 Tier-S style `no_std`/`alloc` building blocks and pure formats. No OS calls except through a sibling foundation crate (`rusty_libc`, `rusty_win32`), no async runtime. | `foundation` |
| `crates/platform/` | OS abstraction: typed capability APIs over NT/Linux/BSD (`rustils`), the async platform layer (`rustils_async`), and the portable runtime contract (today `rusty_test`). | `foundation`, `platform` |
| `crates/libs/` | std-level reusable libraries: async runtime, network and protocol stacks, storage engines and drivers, UI/media stacks, ML inference, client SDKs, test doubles. Thematic subdirectories are used where at least three crates share a theme. | `foundation`, `platform`, `libs` |
| `crates/apps/` | Deployable products: CLIs, daemons, desktop shells, servers, and multi-crate product families. Nothing outside a family depends on an app crate. | everything below |
| `crates/tools/` | Workspace tooling and integration harnesses with no library consumers. | everything |

The layout encodes one invariant: **a crate may only depend on crates in
the same layer or a lower one.** Family-internal edges are unconstrained.
This is checked mechanically (Phase 0 below), so the tree cannot drift
from the graph the way the README prose has.

`libs/` subdirectories in this proposal, each with three or more crates:

| Subdirectory | Families |
|---|---|
| `libs/async/` | `rusty_tokio` (+ `rusty_tokio-macros`), `rusty_stream` |
| `libs/net/` | `rusty_http`, `rusty_h2`, `rusty_tls`, `rusty_request`, `rusty_kafka`, `rusty_rdp`, `rusty_oauth` |
| `libs/protocol/` | `rusty_mcp` (+ `rusty-mcp-demo`), `rusty_a2a`, `rusty_acp`, `rusty_lsp` |
| `libs/storage/` | `rusty_sqlite`, `rusty_rusqlite`, `rusty_db` (6 crates), `rusty_search` (12 crates) |
| `libs/ui/` | `rusty_font`, `rusty_gpu`, `rusty_gui`, `rusty_vulkan`, `rusty_audio`, `rusty_ansi`, `rusty_lines`, `rusty_term` (+ `rusty_term_l13`) |
| `libs/ai/` | `rusty_whisper`, `rusty_llama`, `rusty_rag` |
| `libs/homelab/` | `rusty_proxmox`, `rusty_opnsense`, `rusty_fedora` (REST client libraries; the MCP server that uses all three, `rusty_homelab_mcp`, and the Fedora host agent `rusty_fedora_agent`, which has no Cargo edge to `rusty_fedora`, are apps) |
| `libs/` root | `rusty_git` (library plus `rgit`; `mill-term` depends on the library), `rusty_wiremock` (shared test double for the homelab clients), `rusty_adk` (an SDK; 14 crates incl. `examples/`) |

### Family split rule

A multi-crate family directory stays intact under one layer when it is one
product. It is split only when its reusable part has dependents outside
the family and its application part has none. Applied at this commit:

| Family | Decision | Evidence |
|---|---|---|
| `rustils` | **Split.** `platform*` and `winargv` to `crates/platform/rustils/crates/`; `coreutils` to `crates/apps/coreutils`. rustils' `README.md`, `docs/rfc-v2.md` and `docs/learning/` stay with the platform crates, which is what they govern. | `platform` has 12 external dependents; `coreutils` has none and is a `bin`. |
| `rustils_async` | **Split** the same way: `reactor-core`, `platform-async*`, `threading` to `crates/platform/rustils_async/crates/`; `coreutils-async` to `crates/apps/coreutils-async`. | `platform-async*` are consumed only by `coreutils-async` today, but they are the async half of the same abstraction and belong beside it. |
| `rusty_test` | **Keep intact, place in `platform/`, rename the directory to `portable-runtime`.** `contract`/`compat`/`conformance` are an OS abstraction (the README's own title is `portable-runtime-contract`); `tools/stat-tool`, `proc-runner`, `pty-shell` are its reference tools with no other consumers. | `crates/rusty_test/README.md` lines 1 to 12. No external dependents. |
| `rusty_mcp` | Keep intact in `libs/protocol/`. `rusty-mcp-demo` is an example of the library and has no consumers. | `rusty-mcp` has 6 external dependents (`agentgateway*`, `rp-mcp`, `rp-server`, `rusty_homelab_mcp`). |
| `rusty_adk` | Keep intact in `libs/`, including `examples/*` members. | It is an SDK by purpose; no external dependents yet. |
| `rusty_search`, `rusty_db` | Keep intact in `libs/storage/`. | Library families with backend adapters; `rusty-db` has one external dependent (`rusty-hister-model`). |
| `nexus` | Keep intact in `apps/`. | 42 crates, one product. `nexus-hashline` and `nexus-plugin-api` are `[workspace.dependencies]` entries but no crate outside nexus uses them at this commit. |
| `rusty_tailscale` (incl. `xtask`), `rusty_meshed`, `rusty_key`, `rusty_yirp`, `rusty_provider`, `rusty_agent_gateway`, `rusty_hister`, `rusty_inventrory`, `rusty_skillopt`, `rusty_multimodal_db` | Keep intact in `apps/`. | Product families with binaries; no crate outside the family depends on any member. `rusty_provider`'s `rp-core`/`rp-providers` become a `libs/` move the day another family depends on them, which the Phase 0 check will flag as an app-to-app edge. |
| `rusty_serde`, `rusty_json`, `rusty_err`, `rusty_tokio`, `rusty_term` | Keep intact (a library plus its derive/macro/side-channel crate). | Same reason: one library. |
| `rusty_boot` | `tools/`. Its `src/main.rs` header calls it a "kernel-to-application demonstration binary" that exercises Level 0 to Level 3/4; it depends on 22 workspace crates including the shell `rush` (an app) and the terminal library `rusty_term`. It is an integration harness, not a product. | `crates/rusty_boot/src/main.rs` lines 1 to 3; `crates/rusty_boot/Cargo.toml` lines 13 to 38. |
| `rusty_text`, `rusty_croc`, `rusty_voice`, `rusty_homelab_mcp`, `rusty_fedora_agent`, `rush`, `mill-term` | `apps/`. | Binaries; the only cross-family dependents are `mill-term` (shells out to `rsed`/`rawk`, not a Cargo edge) and `rusty_boot` (tools). |

### Directory and naming conventions

- A single-crate family's directory name equals its crate name. Today 21
  member directories differ from their crate name; the family-prefixed
  ones (`rk-*` under `rusty_key/crates/app|compose|config|...`, `rp-*`
  under `rusty_provider/crates/core|cli|...`) are the ones that mislead.
  Multi-crate families keep `crates/<layer>/<family>/crates/<crate>` and
  their family README lists every crate name whose prefix differs from
  the directory (`rusty_key` holds `rk-*`, `rusty_yirp` holds
  `sessionmgr-*`, `rusty_provider` holds `rp-*`, `rusty_tailscale` holds
  `ts-*`, `portable-runtime` holds `contract`/`compat`/`conformance`).
- `rusty_inventrory` is renamed `rusty_inventory` during its move (a
  directory rename only; the crates are `inventory-*`).
- Generic, unprefixed crate names collide easily with `cargo -p` and with
  anything else that joins the workspace: `platform`, `platform-*`,
  `winargv`, `reactor-core`, `threading`, `contract`, `compat`,
  `conformance`, `coreutils`, `coreutils-async`, `stat-tool`,
  `proc-runner`, `pty-shell`. All of these are `publish = false`
  (`cargo metadata` reports `publish: []` for each), so renaming them
  breaks no published API. `xtask` (`rusty_tailscale`'s build helper) is
  equally generic but has no `publish` key, so it is publishable by
  default and is left out of the rename list. Renaming is **optional
  Phase 5** and is not required for the layout; `platform` to
  `rustils-platform` is the one rename with a clear payoff because it is
  the most-consumed generic name.

### Migration plan

Phases are independently mergeable PRs in this order. Counts were
computed at `cd789d573` by a standard-library Python pass over `cargo
metadata --no-deps --locked` plus a `tomllib` parse of every member
manifest (all of `[dependencies]`, `[dev-dependencies]`,
`[build-dependencies]` and their `[target.*]` variants). Rules: a
"cross-family relative path dep" is a `path = "../..."` entry whose target
is a workspace member in a different top-level family (or a crate this ADR
splits out of its family), so the relative path stops resolving once
either endpoint moves; a manifest is counted once, at the first phase
that breaks one of its entries. 128 member manifests carry at least one
relative `path` entry; 85 of them carry at least one cross-family entry
(257 entries in total), and those 85 are the Phase 0b set.

| Phase | Scope | Members moved | Member manifests to rewrite | `[workspace.dependencies]` path entries to repoint |
|---|---|---|---|---|
| 0a | Metadata + check + generated map (no moves) | 0 | 239 (one `[package.metadata.rusty_mill]` table each) | 0 |
| 0b | Hoist every cross-family `path = "../..."` dependency into `[workspace.dependencies]` (`dep.workspace = true` in members) | 0 | 85 (257 dependency entries) | new entries added |
| 1 | `platform/`: `rustils` (split), `rustils_async` (split), `rusty_test` to `portable-runtime` | 18 | 9 (0 after 0b) | 10 |
| 2 | `foundation/`: 26 families | 30 | 66 (0 after 0b) | 5 |
| 3 | `libs/`: 34 families | 66 | 10 (0 after 0b) | 30 |
| 4 | `apps/` and `tools/` | 125 | 0 (0 after 0b) | 45 |
| 5 | Optional renames of generic crate names | 0 | per crate | per crate |

Phase 0b is what makes phases 1 to 4 cheap: after it, each move is a
`git mv`, a root `Cargo.toml` edit (member paths, `[workspace.dependencies]`
paths, `exclude` paths) and documentation links, with no member manifest
touched. Without 0b, Phase 2 alone rewrites 66 manifests because the
foundation crates are the most depended-on (and Phase 3 would touch
many more than 10 if it ran before Phase 2, because app manifests that
reach a libs crate by relative path break again at that point).

**Phase 0a in detail.**

1. Every member gets
   ```toml
   [package.metadata.rusty_mill]
   layer = "foundation"   # foundation | platform | libs | apps | tools
   ```
   and optionally `tier = "S"` (ADR-0002's S/T/A), so the classification
   lives next to the code and survives any later move.
2. A checker in `.github/scripts/` (a sibling of
   `check_workspace_deps.py`, with `test_*.py` coverage picked up by the
   existing `plan-tests` job) reads `cargo metadata --all-features`, fails
   on any member without a `layer`, fails on any path edge that points to
   a higher layer, and — restricted to callers whose own layer is
   `apps` — fails on any edge to an `apps` crate in another family (this
   is the `apps` row's "nothing outside a family depends on an app crate"
   rule, not a general cross-family prohibition). A `tools` caller is
   exempt from that second check because the `tools` row already permits
   depending on everything; `crates/rusty_boot/Cargo.toml`'s
   `rush = { path = "../rush" }` (tools → apps, cross-family) is the
   existing edge this exemption covers, and the checker's `test_*.py`
   suite includes it as an accepted case so the exemption can't silently
   widen later. It is wired into the existing `dependency-policy` job.
3. A generated `docs/WORKSPACE-MAP.md` (layer, family, crate, one-line
   description from `[package] description`, dependents count) replaces
   the hand-maintained "How the crates relate" narrative in `README.md`;
   the README keeps its history section and links to the generated map.
   CI fails if the committed map is stale, the way nexus's
   `check_ipc_drift.sh` already works for its IPC schemas.

**What each move phase touches besides `git mv`.**

- Root `Cargo.toml`: `[workspace] members` paths, `exclude` paths (the 10
  entries today: `crates/rusty_term/fuzz`, `crates/rush/fuzz`,
  `crates/rusty_lines/bench`, `crates/rusty_libc/bench`,
  `crates/rusty_tls/fuzz`, `crates/rusty_lsp/fuzz`, `crates/rustils/fuzz`,
  `crates/rusty_croc/fuzz`, `crates/rusty_key/desktop/src-tauri`,
  `crates/nexus/shell`; fuzz and bench directories move with their family),
  and every `[workspace.dependencies]` entry with `path = "crates/..."`.
  `.github/scripts/cargo_toml_diff.py` classifies any member-path edit as
  unsafe, so each phase runs the full CI sweep. That is correct.
- Three stale nested `[workspace]` manifests are still tracked and are
  not workspace members: `crates/rustils/Cargo.toml` (its own `members`
  list and `[workspace.dependencies]` path table, lines 32 to 38),
  `crates/rustils_async/Cargo.toml` (lines 36 to 38 point at
  `../rustils/crates/platform*` and break at Phase 1) and
  `crates/rusty_serde/Cargo.toml`. Cargo resolves the nearest ancestor
  manifest that has a `[workspace]` table, so a `cargo` command run from
  inside one of those directories uses the stale nested root instead of
  this workspace. Delete all three in Phase 1 (rustils, rustils_async) and
  Phase 2 (rusty_serde); the root workspace is authoritative.
- `Cargo.lock` does not change: path members are recorded by name and
  version with no `source` field (`Cargo.lock` lines 13126 to 13130 for
  `rusty_std`, 10001 to 10004 for `platform`).
- `.github/scripts/affected_crates.py` needs no change: it derives crate
  directories from `manifest_path` in `cargo metadata`.
- Hard-coded paths to update: `.github/workflows/ci.yml` (the
  `data-mesh-monitor` change filter at line 162 and the job's
  `working-directory`/`cache-dependency-path` at lines 420 and 427; the
  comments at lines 49, 433, 458), `.github/actions/setup-build-env/
  action.yml` (comment at line 55). `.config/nextest.toml` has no paths.
- Living documents: `README.md` crate table (239 rows with `crates/...`
  links), `ARCHITECTURE.md`, `CONTRIBUTING.md` (no paths today),
  `docs/adr/0001` and `0002` (they cite `crates/<name>` paths as history;
  add a one-line "paths as of the date" note rather than rewriting them).
- Per-crate documents: of 1451 markdown files under `crates/`, 27
  relative links in 11 READMEs cross a family boundary and break on a
  move (`mill-term`, `rusty_ansder`, `rusty_fedora`, `rusty_fedora_agent`,
  `rusty_git`, `rusty_homelab_mcp`, `rusty_meshed`, `rusty_opnsense`,
  `rusty_proxmox`, `rusty_rag`, `rusty_text`); family-internal relative
  links survive because families move as a unit; 408 files mention a
  `crates/<name>/` path in prose, which is not a link and needs no edit.
  Fix the 27 with the move.
- Rust test consumers that hardcode a crate-group's current path. At
  Phase 1, `crates/rusty_test/crates/conformance/tests/layering.rs` sets
  `GROUP_PREFIX = "crates/rusty_test/"` and finds the workspace root via
  `CARGO_MANIFEST_DIR.ancestors().nth(4)`, a depth that is only correct
  before the move. At Phase 4, `rg -F '"crates/nexus' --type rust
  crates/nexus` (re-run at move time; this ADR's own count, from that
  same search at `cd789d573`, is seven files) finds every Nexus guard
  under
  `crates/nexus/crates/nexus-bootstrap/tests/` that hardcodes
  `"crates/nexus/crates"` and/or `"crates/nexus/shell"` as the member-path
  prefix it filters `[workspace] members` or walks the filesystem
  against: `bootstrap_coverage.rs`, `core_plugin_loc_budget.rs`,
  `dep_invariants.rs`, `dep_invariants_shell.rs`,
  `ipc_topic_prefix_invariant.rs`, `plugin_contract_purity.rs` and
  `tauri_command_boundary.rs`. Six of the seven already walk up from
  `CARGO_MANIFEST_DIR` to find the workspace root dynamically by looking
  for the ancestor whose `Cargo.toml` has a `[workspace]` table (only
  `layering.rs` uses a fixed depth), so only that one needs the
  root-finding fix, but every file in both sets needs its hardcoded
  family-prefix string literal updated to the crate's new path
  (`crates/platform/portable-runtime/` and `crates/apps/nexus/`,
  respectively). Fix in the same PR as the move: replace the fixed
  `ancestors().nth(N)` in `layering.rs` with the nearest-ancestor-with-a-
  `[workspace]`-table walk the Nexus guards already use, update every
  hardcoded prefix constant, and re-run the same `rg -F` search after the
  `git mv` to confirm no consumer of the old literal remains — do not
  treat the file list above as exhaustive for a family this ADR did not
  already grep. Keep each file's nonempty-member-list / missing-path
  assertions as is; a filter that silently matches zero members or an
  absent directory after a move must still fail loudly, not pass by
  accident.
- Historical documents are not rewritten: `CODEX-MONOREPO-REVIEW*.md`,
  `RELEASE_NOTES.md`, `repo-inspector-report.md`, `docs/atlas/*`,
  per-crate `CHANGELOG.md`/`RELEASE_NOTES.md`.
- `git subtree` history (ADR-0001): `git mv` keeps per-file history
  reachable through `git log --follow`; a future `git subtree pull` would
  need the new `--prefix`. Every merged upstream repo is archived
  (`README.md`, "History"), so no pull is expected.
- External links into `https://github.com/Rusty-Mill/rusty_mill/tree/main/crates/<name>`
  break. Add a short "moved paths" table to `README.md` at Phase 1 and
  keep it until Phase 4 lands.

### Enforcement after the move

The Phase 0a checker is the durable part of this decision. The directory
layout is the human-facing rendering of the same fact; the checker is
what keeps the two from disagreeing. A new family joining the workspace
must declare its `layer` and land under the matching directory, or CI
fails.

## Alternatives considered

**Metadata only, no directory moves (Phase 0a alone).** Delivers the
mechanical check and the generated map for nearly zero churn, and it is
the recommended first PR regardless. It does not fix the thing the user
of this tree sees first: `ls crates/` still shows 82 undifferentiated
names, and `platform` still lives under `rustils`. Recommended as the
first step, not as the destination.

**Group by domain instead of by layer** (`crates/terminal/`,
`crates/agents/`, `crates/homelab/`, ...). Reads well for products but
puts `rusty_tokio` and `rusty_std` nowhere natural, and a domain grouping
cannot be checked against the dependency graph, which is the property
that stops the layout from rotting. Themes survive as `libs/`
subdirectories, where they are the natural second axis.

**One nested Cargo workspace per family.** Cargo does not support nested
workspaces: the root `Cargo.toml` comments record how `rusty_search`,
`rusty_db`, `rustils`, `rusty_test`, `rusty_key`, `rusty_tailscale`,
`rusty_adk`, `rusty_provider`, `rusty_yirp`, `rusty_agent_gateway` and
`nexus` had their `[workspace]` tables hoisted or de-inherited to join
(three stale nested manifests survive as tracked files, see the migration
plan), and splitting back into several workspaces reintroduces the
multi-lockfile drift ADR-0001 consolidated away.

**Split the repo** into a foundation repo and an applications repo. Loses
the single CI run across a foundation change and its consumers, which is
the whole point of ADR-0001.

**Rename every crate with a uniform prefix** (`rusty-*`) at the same time.
Most crates are publishable identities carried over from their own repos
and the README documents many of them by name; renaming is a separate
decision with API cost and is confined to the optional Phase 5 for
unpublished generic names only.

## Consequences

- Positive: a newcomer can read the layer from the path; the
  reusable/product distinction is explicit; the OS abstraction layer is
  found under `platform/`, not under a consumer; the dependency direction
  becomes a CI rule instead of a paragraph in `ARCHITECTURE.md`; new
  families have a defined place to land.
- Cost: four move PRs, each a full CI sweep; 27 cross-family relative
  doc links and the CI/README paths above; external URL breakage for the
  duration of the migration; `git log` without `--follow` shows a rename
  boundary at each move.
- The layer assignment is a judgment in a handful of places, recorded
  here so they can be revisited without re-deriving the rest:
  `rusty_boot` as tools rather than apps; `rusty_git` and `rusty_adk` in
  `libs/` despite having (almost) no in-workspace dependents; `rusty_term`
  (a library with four binaries and two dependents, `mill-term` and
  `rusty_boot`) in `libs/ui/` rather than apps; the homelab REST clients
  in `libs/` while their MCP server and agent are apps; `rusty_h2`,
  `rusty_rdp` and `rusty_oauth` in `libs/net/` although they have no
  dependents yet; `rusty_h2` and `rusty_oauth` in `libs/net/` by theme
  although each also meets the foundation definition (no OS, no runtime,
  zero external dependencies); `rusty_ansi` in `libs/ui/` beside
  `rusty_term` (ARCHITECTURE.md's terminal/shell cluster).
- ARCHITECTURE.md's "Boundaries" table is superseded by this ADR's layer
  table once Phase 0a lands.

## Objections recorded during drafting

None from a second builder: Codex could not run (see the authorship note).
The judgment calls listed in the last Consequences bullet are the places
to push on; none of them changes the invariant or the checker.

An independent Codex review of this ADR (once Codex could run from this
host, via `/codex-build` review mode) is recorded in full in
`PLAN-REVIEW-LOG.md`. Round 1 returned REVISE with two high findings,
both accepted: the move-phase inventory omitted the Rust test files that
hardcode a crate-group's current path, and the Phase 0a checker's
app-to-app rule as originally worded would have rejected `rusty_boot`'s
existing, plan-permitted `rush` dependency. Round 2 confirmed the checker
fix closed that second finding but found the first fix incomplete —
three more Nexus guards hardcode the same family-path literal — so the
move-phase inventory (now under "What each move phase touches besides
`git mv`") lists all seven Nexus files plus `layering.rs`, and specifies
a re-run `rg` sweep at move time instead of treating any fixed list as
exhaustive.

## Appendix A: layer order

```layer-order
foundation
platform
libs
apps
tools
```

## Appendix B: layer map

One line per workspace member, tab-separated:
`crate-name`, `layer`, current directory, proposed directory. Generated
from `cargo metadata --no-deps --locked` at `cd789d573` by the same rules
as the tables above; the review's proof script re-derives the dependency
direction check from this block.

```layer-map
rpath	foundation	crates/rpath	crates/foundation/rpath
rusty_ansder	foundation	crates/rusty_ansder	crates/foundation/rusty_ansder
rusty_base64	foundation	crates/rusty_base64	crates/foundation/rusty_base64
rusty_codec	foundation	crates/rusty_codec	crates/foundation/rusty_codec
rusty_compress	foundation	crates/rusty_compress	crates/foundation/rusty_compress
rusty_config	foundation	crates/rusty_config	crates/foundation/rusty_config
rusty_crypto_key	foundation	crates/rusty_crypto_key	crates/foundation/rusty_crypto_key
rusty_diff	foundation	crates/rusty_diff	crates/foundation/rusty_diff
rusty_err	foundation	crates/rusty_err	crates/foundation/rusty_err
rusty_err_derive	foundation	crates/rusty_err/derive	crates/foundation/rusty_err/derive
rusty_jinja	foundation	crates/rusty_jinja	crates/foundation/rusty_jinja
rusty_json	foundation	crates/rusty_json	crates/foundation/rusty_json
rusty_json-derive	foundation	crates/rusty_json/rusty_json-derive	crates/foundation/rusty_json/rusty_json-derive
rusty_libc	foundation	crates/rusty_libc	crates/foundation/rusty_libc
rusty_rand	foundation	crates/rusty_rand	crates/foundation/rusty_rand
rusty_regx	foundation	crates/rusty_regx	crates/foundation/rusty_regx
rusty_retry	foundation	crates/rusty_retry	crates/foundation/rusty_retry
rusty_rsa	foundation	crates/rusty_rsa	crates/foundation/rusty_rsa
rusty_serde	foundation	crates/rusty_serde/rusty_serde	crates/foundation/rusty_serde/rusty_serde
rusty_serde_derive	foundation	crates/rusty_serde/rusty_serde_derive	crates/foundation/rusty_serde/rusty_serde_derive
rusty_serde_erased	foundation	crates/rusty_serde/rusty_serde_erased	crates/foundation/rusty_serde/rusty_serde_erased
rusty_sha1	foundation	crates/rusty_sha1	crates/foundation/rusty_sha1
rusty_simd	foundation	crates/rusty_simd	crates/foundation/rusty_simd
rusty_std	foundation	crates/rusty_std	crates/foundation/rusty_std
rusty_sync	foundation	crates/rusty_sync	crates/foundation/rusty_sync
rusty_time	foundation	crates/rusty_time	crates/foundation/rusty_time
rusty_url	foundation	crates/rusty_url	crates/foundation/rusty_url
rusty_uuid	foundation	crates/rusty_uuid	crates/foundation/rusty_uuid
rusty_win32	foundation	crates/rusty_win32	crates/foundation/rusty_win32
rusty_wire	foundation	crates/rusty_wire	crates/foundation/rusty_wire
compat	platform	crates/rusty_test/crates/compat	crates/platform/portable-runtime/crates/compat
conformance	platform	crates/rusty_test/crates/conformance	crates/platform/portable-runtime/crates/conformance
contract	platform	crates/rusty_test/crates/contract	crates/platform/portable-runtime/crates/contract
proc-runner	platform	crates/rusty_test/tools/proc-runner	crates/platform/portable-runtime/tools/proc-runner
pty-shell	platform	crates/rusty_test/tools/pty-shell	crates/platform/portable-runtime/tools/pty-shell
stat-tool	platform	crates/rusty_test/tools/stat-tool	crates/platform/portable-runtime/tools/stat-tool
platform	platform	crates/rustils/crates/platform	crates/platform/rustils/crates/platform
platform-bsd	platform	crates/rustils/crates/platform-bsd	crates/platform/rustils/crates/platform-bsd
platform-linux	platform	crates/rustils/crates/platform-linux	crates/platform/rustils/crates/platform-linux
platform-mock	platform	crates/rustils/crates/platform-mock	crates/platform/rustils/crates/platform-mock
platform-parity	platform	crates/rustils/crates/platform-parity	crates/platform/rustils/crates/platform-parity
platform-windows	platform	crates/rustils/crates/platform-windows	crates/platform/rustils/crates/platform-windows
winargv	platform	crates/rustils/crates/winargv	crates/platform/rustils/crates/winargv
platform-async	platform	crates/rustils_async/crates/platform-async	crates/platform/rustils_async/crates/platform-async
platform-async-linux	platform	crates/rustils_async/crates/platform-async-linux	crates/platform/rustils_async/crates/platform-async-linux
platform-async-mock	platform	crates/rustils_async/crates/platform-async-mock	crates/platform/rustils_async/crates/platform-async-mock
reactor-core	platform	crates/rustils_async/crates/reactor-core	crates/platform/rustils_async/crates/reactor-core
threading	platform	crates/rustils_async/crates/threading	crates/platform/rustils_async/crates/threading
rusty_llama	libs	crates/rusty_llama	crates/libs/ai/rusty_llama
rusty_rag	libs	crates/rusty_rag	crates/libs/ai/rusty_rag
rusty-whisper	libs	crates/rusty_whisper	crates/libs/ai/rusty_whisper
rusty_stream	libs	crates/rusty_stream	crates/libs/async/rusty_stream
rusty_tokio	libs	crates/rusty_tokio	crates/libs/async/rusty_tokio
rusty_tokio-macros	libs	crates/rusty_tokio/rusty_tokio-macros	crates/libs/async/rusty_tokio/rusty_tokio-macros
rusty_fedora	libs	crates/rusty_fedora	crates/libs/homelab/rusty_fedora
rusty_opnsense	libs	crates/rusty_opnsense	crates/libs/homelab/rusty_opnsense
rusty_proxmox	libs	crates/rusty_proxmox	crates/libs/homelab/rusty_proxmox
rusty_h2	libs	crates/rusty_h2	crates/libs/net/rusty_h2
rusty_http	libs	crates/rusty_http	crates/libs/net/rusty_http
rusty_kafka	libs	crates/rusty_kafka	crates/libs/net/rusty_kafka
rusty_oauth	libs	crates/rusty_oauth	crates/libs/net/rusty_oauth
rusty_rdp	libs	crates/rusty_rdp	crates/libs/net/rusty_rdp
rusty_request	libs	crates/rusty_request	crates/libs/net/rusty_request
rusty_tls	libs	crates/rusty_tls	crates/libs/net/rusty_tls
rusty_a2a	libs	crates/rusty_a2a	crates/libs/protocol/rusty_a2a
rusty-acp	libs	crates/rusty_acp	crates/libs/protocol/rusty_acp
rusty_lsp	libs	crates/rusty_lsp	crates/libs/protocol/rusty_lsp
rusty-mcp	libs	crates/rusty_mcp/crates/rusty-mcp	crates/libs/protocol/rusty_mcp/crates/rusty-mcp
rusty-mcp-demo	libs	crates/rusty_mcp/crates/rusty-mcp-demo	crates/libs/protocol/rusty_mcp/crates/rusty-mcp-demo
adk-a2a	libs	crates/rusty_adk/crates/adk-a2a	crates/libs/rusty_adk/crates/adk-a2a
adk-agents	libs	crates/rusty_adk/crates/adk-agents	crates/libs/rusty_adk/crates/adk-agents
adk-core	libs	crates/rusty_adk/crates/adk-core	crates/libs/rusty_adk/crates/adk-core
adk-graph	libs	crates/rusty_adk/crates/adk-graph	crates/libs/rusty_adk/crates/adk-graph
adk-macros	libs	crates/rusty_adk/crates/adk-macros	crates/libs/rusty_adk/crates/adk-macros
adk-mcp	libs	crates/rusty_adk/crates/adk-mcp	crates/libs/rusty_adk/crates/adk-mcp
adk-models	libs	crates/rusty_adk/crates/adk-models	crates/libs/rusty_adk/crates/adk-models
adk-runner	libs	crates/rusty_adk/crates/adk-runner	crates/libs/rusty_adk/crates/adk-runner
adk-sessions	libs	crates/rusty_adk/crates/adk-sessions	crates/libs/rusty_adk/crates/adk-sessions
adk-tools	libs	crates/rusty_adk/crates/adk-tools	crates/libs/rusty_adk/crates/adk-tools
rusty-adk	libs	crates/rusty_adk/crates/rusty-adk	crates/libs/rusty_adk/crates/rusty-adk
a2a-agent-server	libs	crates/rusty_adk/examples/a2a-agent-server	crates/libs/rusty_adk/examples/a2a-agent-server
mcp-tool-server	libs	crates/rusty_adk/examples/mcp-tool-server	crates/libs/rusty_adk/examples/mcp-tool-server
weather-agent	libs	crates/rusty_adk/examples/weather-agent	crates/libs/rusty_adk/examples/weather-agent
rusty_git	libs	crates/rusty_git	crates/libs/rusty_git
rusty_wiremock	libs	crates/rusty_wiremock	crates/libs/rusty_wiremock
rusty-db-core	libs	crates/rusty_db/crates/rusty-db-core	crates/libs/storage/rusty_db/crates/rusty-db-core
rusty-db-derive	libs	crates/rusty_db/crates/rusty-db-derive	crates/libs/storage/rusty_db/crates/rusty-db-derive
rusty-db-mysql	libs	crates/rusty_db/crates/rusty-db-mysql	crates/libs/storage/rusty_db/crates/rusty-db-mysql
rusty-db-postgres	libs	crates/rusty_db/crates/rusty-db-postgres	crates/libs/storage/rusty_db/crates/rusty-db-postgres
rusty-db-sqlite	libs	crates/rusty_db/crates/rusty-db-sqlite	crates/libs/storage/rusty_db/crates/rusty-db-sqlite
rusty-db	libs	crates/rusty_db/rusty_db	crates/libs/storage/rusty_db/rusty_db
rusty_rusqlite	libs	crates/rusty_rusqlite	crates/libs/storage/rusty_rusqlite
rusty-search	libs	crates/rusty_search/crates/rusty-search	crates/libs/storage/rusty_search/crates/rusty-search
rusty-search-algolia	libs	crates/rusty_search/crates/rusty-search-algolia	crates/libs/storage/rusty_search/crates/rusty-search-algolia
rusty-search-azure-search	libs	crates/rusty_search/crates/rusty-search-azure-search	crates/libs/storage/rusty_search/crates/rusty-search-azure-search
rusty-search-cloud	libs	crates/rusty_search/crates/rusty-search-cloud	crates/libs/storage/rusty_search/crates/rusty-search-cloud
rusty-search-core	libs	crates/rusty_search/crates/rusty-search-core	crates/libs/storage/rusty_search/crates/rusty-search-core
rusty-search-elasticsearch	libs	crates/rusty_search/crates/rusty-search-elasticsearch	crates/libs/storage/rusty_search/crates/rusty-search-elasticsearch
rusty-search-meilisearch	libs	crates/rusty_search/crates/rusty-search-meilisearch	crates/libs/storage/rusty_search/crates/rusty-search-meilisearch
rusty-search-memory	libs	crates/rusty_search/crates/rusty-search-memory	crates/libs/storage/rusty_search/crates/rusty-search-memory
rusty-search-opensearch	libs	crates/rusty_search/crates/rusty-search-opensearch	crates/libs/storage/rusty_search/crates/rusty-search-opensearch
rusty-search-solr	libs	crates/rusty_search/crates/rusty-search-solr	crates/libs/storage/rusty_search/crates/rusty-search-solr
rusty-search-sqlite-fts5	libs	crates/rusty_search/crates/rusty-search-sqlite-fts5	crates/libs/storage/rusty_search/crates/rusty-search-sqlite-fts5
rusty-search-tantivy	libs	crates/rusty_search/crates/rusty-search-tantivy	crates/libs/storage/rusty_search/crates/rusty-search-tantivy
rusty_sqlite	libs	crates/rusty_sqlite	crates/libs/storage/rusty_sqlite
rusty_ansi	libs	crates/rusty_ansi	crates/libs/ui/rusty_ansi
rusty_audio	libs	crates/rusty_audio	crates/libs/ui/rusty_audio
rusty_font	libs	crates/rusty_font	crates/libs/ui/rusty_font
rusty_gpu	libs	crates/rusty_gpu	crates/libs/ui/rusty_gpu
rusty_gui	libs	crates/rusty_gui	crates/libs/ui/rusty_gui
rusty_lines	libs	crates/rusty_lines	crates/libs/ui/rusty_lines
rusty_term	libs	crates/rusty_term	crates/libs/ui/rusty_term
rusty_term_l13	libs	crates/rusty_term/l13	crates/libs/ui/rusty_term/l13
rusty_vulkan	libs	crates/rusty_vulkan	crates/libs/ui/rusty_vulkan
coreutils	apps	crates/rustils/crates/coreutils	crates/apps/coreutils
coreutils-async	apps	crates/rustils_async/crates/coreutils-async	crates/apps/coreutils-async
mill-term	apps	crates/mill-term	crates/apps/mill-term
nexus-acp	apps	crates/nexus/crates/nexus-acp	crates/apps/nexus/crates/nexus-acp
nexus-agent	apps	crates/nexus/crates/nexus-agent	crates/apps/nexus/crates/nexus-agent
nexus-ai	apps	crates/nexus/crates/nexus-ai	crates/apps/nexus/crates/nexus-ai
nexus-ai-runtime	apps	crates/nexus/crates/nexus-ai-runtime	crates/apps/nexus/crates/nexus-ai-runtime
nexus-audio	apps	crates/nexus/crates/nexus-audio	crates/apps/nexus/crates/nexus-audio
nexus-bootstrap	apps	crates/nexus/crates/nexus-bootstrap	crates/apps/nexus/crates/nexus-bootstrap
nexus-cli	apps	crates/nexus/crates/nexus-cli	crates/apps/nexus/crates/nexus-cli
nexus-collab	apps	crates/nexus/crates/nexus-collab	crates/apps/nexus/crates/nexus-collab
nexus-comments	apps	crates/nexus/crates/nexus-comments	crates/apps/nexus/crates/nexus-comments
nexus-context	apps	crates/nexus/crates/nexus-context	crates/apps/nexus/crates/nexus-context
nexus-crdt	apps	crates/nexus/crates/nexus-crdt	crates/apps/nexus/crates/nexus-crdt
nexus-dap	apps	crates/nexus/crates/nexus-dap	crates/apps/nexus/crates/nexus-dap
nexus-database	apps	crates/nexus/crates/nexus-database	crates/apps/nexus/crates/nexus-database
nexus-editor	apps	crates/nexus/crates/nexus-editor	crates/apps/nexus/crates/nexus-editor
nexus-formats	apps	crates/nexus/crates/nexus-formats	crates/apps/nexus/crates/nexus-formats
nexus-fuzz	apps	crates/nexus/crates/nexus-fuzz	crates/apps/nexus/crates/nexus-fuzz
nexus-git	apps	crates/nexus/crates/nexus-git	crates/apps/nexus/crates/nexus-git
nexus-hashline	apps	crates/nexus/crates/nexus-hashline	crates/apps/nexus/crates/nexus-hashline
nexus-kernel	apps	crates/nexus/crates/nexus-kernel	crates/apps/nexus/crates/nexus-kernel
nexus-kv	apps	crates/nexus/crates/nexus-kv	crates/apps/nexus/crates/nexus-kv
nexus-linkpreview	apps	crates/nexus/crates/nexus-linkpreview	crates/apps/nexus/crates/nexus-linkpreview
nexus-lsp	apps	crates/nexus/crates/nexus-lsp	crates/apps/nexus/crates/nexus-lsp
nexus-mcp	apps	crates/nexus/crates/nexus-mcp	crates/apps/nexus/crates/nexus-mcp
nexus-memory	apps	crates/nexus/crates/nexus-memory	crates/apps/nexus/crates/nexus-memory
nexus-memory-hub	apps	crates/nexus/crates/nexus-memory-hub	crates/apps/nexus/crates/nexus-memory-hub
nexus-notifications	apps	crates/nexus/crates/nexus-notifications	crates/apps/nexus/crates/nexus-notifications
nexus-panic-log	apps	crates/nexus/crates/nexus-panic-log	crates/apps/nexus/crates/nexus-panic-log
nexus-plugin-api	apps	crates/nexus/crates/nexus-plugin-api	crates/apps/nexus/crates/nexus-plugin-api
nexus-plugins	apps	crates/nexus/crates/nexus-plugins	crates/apps/nexus/crates/nexus-plugins
nexus-protocol	apps	crates/nexus/crates/nexus-protocol	crates/apps/nexus/crates/nexus-protocol
nexus-remote	apps	crates/nexus/crates/nexus-remote	crates/apps/nexus/crates/nexus-remote
nexus-rush	apps	crates/nexus/crates/nexus-rush	crates/apps/nexus/crates/nexus-rush
nexus-security	apps	crates/nexus/crates/nexus-security	crates/apps/nexus/crates/nexus-security
nexus-skills	apps	crates/nexus/crates/nexus-skills	crates/apps/nexus/crates/nexus-skills
nexus-storage	apps	crates/nexus/crates/nexus-storage	crates/apps/nexus/crates/nexus-storage
nexus-templates	apps	crates/nexus/crates/nexus-templates	crates/apps/nexus/crates/nexus-templates
nexus-terminal	apps	crates/nexus/crates/nexus-terminal	crates/apps/nexus/crates/nexus-terminal
nexus-theme	apps	crates/nexus/crates/nexus-theme	crates/apps/nexus/crates/nexus-theme
nexus-tui	apps	crates/nexus/crates/nexus-tui	crates/apps/nexus/crates/nexus-tui
nexus-types	apps	crates/nexus/crates/nexus-types	crates/apps/nexus/crates/nexus-types
nexus-vt	apps	crates/nexus/crates/nexus-vt	crates/apps/nexus/crates/nexus-vt
nexus-workflow	apps	crates/nexus/crates/nexus-workflow	crates/apps/nexus/crates/nexus-workflow
rush	apps	crates/rush	crates/apps/rush
agentgateway	apps	crates/rusty_agent_gateway/crates/agentgateway	crates/apps/rusty_agent_gateway/crates/agentgateway
agentgateway-a2a	apps	crates/rusty_agent_gateway/crates/agentgateway-a2a	crates/apps/rusty_agent_gateway/crates/agentgateway-a2a
agentgateway-auth	apps	crates/rusty_agent_gateway/crates/agentgateway-auth	crates/apps/rusty_agent_gateway/crates/agentgateway-auth
agentgateway-config	apps	crates/rusty_agent_gateway/crates/agentgateway-config	crates/apps/rusty_agent_gateway/crates/agentgateway-config
agentgateway-core	apps	crates/rusty_agent_gateway/crates/agentgateway-core	crates/apps/rusty_agent_gateway/crates/agentgateway-core
agentgateway-llm	apps	crates/rusty_agent_gateway/crates/agentgateway-llm	crates/apps/rusty_agent_gateway/crates/agentgateway-llm
agentgateway-mcp	apps	crates/rusty_agent_gateway/crates/agentgateway-mcp	crates/apps/rusty_agent_gateway/crates/agentgateway-mcp
agentgateway-proxy	apps	crates/rusty_agent_gateway/crates/agentgateway-proxy	crates/apps/rusty_agent_gateway/crates/agentgateway-proxy
agentgateway-tls	apps	crates/rusty_agent_gateway/crates/agentgateway-tls	crates/apps/rusty_agent_gateway/crates/agentgateway-tls
rusty-croc	apps	crates/rusty_croc	crates/apps/rusty_croc
rusty_fedora_agent	apps	crates/rusty_fedora_agent	crates/apps/rusty_fedora_agent
rusty-hister-core	apps	crates/rusty_hister/crates/rusty-hister-core	crates/apps/rusty_hister/crates/rusty-hister-core
rusty-hister-crawler	apps	crates/rusty_hister/crates/rusty-hister-crawler	crates/apps/rusty_hister/crates/rusty-hister-crawler
rusty-hister-extractor	apps	crates/rusty_hister/crates/rusty-hister-extractor	crates/apps/rusty_hister/crates/rusty-hister-extractor
rusty-hister-indexer	apps	crates/rusty_hister/crates/rusty-hister-indexer	crates/apps/rusty_hister/crates/rusty-hister-indexer
rusty-hister-mcp	apps	crates/rusty_hister/crates/rusty-hister-mcp	crates/apps/rusty_hister/crates/rusty-hister-mcp
rusty-hister-model	apps	crates/rusty_hister/crates/rusty-hister-model	crates/apps/rusty_hister/crates/rusty-hister-model
rusty-hister-server	apps	crates/rusty_hister/crates/rusty-hister-server	crates/apps/rusty_hister/crates/rusty-hister-server
rusty-hister-vectorstore	apps	crates/rusty_hister/crates/rusty-hister-vectorstore	crates/apps/rusty_hister/crates/rusty-hister-vectorstore
rusty_homelab_mcp	apps	crates/rusty_homelab_mcp	crates/apps/rusty_homelab_mcp
inventory-cli	apps	crates/rusty_inventrory/crates/inventory-cli	crates/apps/rusty_inventory/crates/inventory-cli
inventory-core	apps	crates/rusty_inventrory/crates/inventory-core	crates/apps/rusty_inventory/crates/inventory-core
inventory-tauri	apps	crates/rusty_inventrory/crates/inventory-tauri	crates/apps/rusty_inventory/crates/inventory-tauri
rk-app	apps	crates/rusty_key/crates/app	crates/apps/rusty_key/crates/app
rk-compose	apps	crates/rusty_key/crates/compose	crates/apps/rusty_key/crates/compose
rk-config	apps	crates/rusty_key/crates/config	crates/apps/rusty_key/crates/config
rk-constrain	apps	crates/rusty_key/crates/constrain	crates/apps/rusty_key/crates/constrain
rk-feed	apps	crates/rusty_key/crates/feed	crates/apps/rusty_key/crates/feed
rk-kernel	apps	crates/rusty_key/crates/kernel	crates/apps/rusty_key/crates/kernel
rk-mcp	apps	crates/rusty_key/crates/mcp	crates/apps/rusty_key/crates/mcp
rk-observe	apps	crates/rusty_key/crates/observe	crates/apps/rusty_key/crates/observe
rusty-meshed-cli	apps	crates/rusty_meshed/crates/rusty-meshed-cli	crates/apps/rusty_meshed/crates/rusty-meshed-cli
rusty-meshed-core	apps	crates/rusty_meshed/crates/rusty-meshed-core	crates/apps/rusty_meshed/crates/rusty-meshed-core
rusty-meshed-domains	apps	crates/rusty_meshed/crates/rusty-meshed-domains	crates/apps/rusty_meshed/crates/rusty-meshed-domains
rusty-meshed-governance	apps	crates/rusty_meshed/crates/rusty-meshed-governance	crates/apps/rusty_meshed/crates/rusty-meshed-governance
rusty-meshed-observability	apps	crates/rusty_meshed/crates/rusty-meshed-observability	crates/apps/rusty_meshed/crates/rusty-meshed-observability
rusty-meshed-registry	apps	crates/rusty_meshed/crates/rusty-meshed-registry	crates/apps/rusty_meshed/crates/rusty-meshed-registry
rusty-meshed-schema-registry	apps	crates/rusty_meshed/crates/rusty-meshed-schema-registry	crates/apps/rusty_meshed/crates/rusty-meshed-schema-registry
rusty-meshed-sdk	apps	crates/rusty_meshed/crates/rusty-meshed-sdk	crates/apps/rusty_meshed/crates/rusty-meshed-sdk
rusty-meshed-trace	apps	crates/rusty_meshed/crates/rusty-meshed-trace	crates/apps/rusty_meshed/crates/rusty-meshed-trace
rusty_multimodal_db	apps	crates/rusty_multimodal_db	crates/apps/rusty_multimodal_db
rp-cli	apps	crates/rusty_provider/crates/cli	crates/apps/rusty_provider/crates/cli
rp-core	apps	crates/rusty_provider/crates/core	crates/apps/rusty_provider/crates/core
rp-mcp	apps	crates/rusty_provider/crates/mcp	crates/apps/rusty_provider/crates/mcp
rp-providers	apps	crates/rusty_provider/crates/providers	crates/apps/rusty_provider/crates/providers
rp-router	apps	crates/rusty_provider/crates/router	crates/apps/rusty_provider/crates/router
rp-server	apps	crates/rusty_provider/crates/server	crates/apps/rusty_provider/crates/server
skillopt-cli	apps	crates/rusty_skillopt/crates/skillopt-cli	crates/apps/rusty_skillopt/crates/skillopt-cli
skillopt-core	apps	crates/rusty_skillopt/crates/skillopt-core	crates/apps/rusty_skillopt/crates/skillopt-core
skillopt-envs	apps	crates/rusty_skillopt/crates/skillopt-envs	crates/apps/rusty_skillopt/crates/skillopt-envs
skillopt-model	apps	crates/rusty_skillopt/crates/skillopt-model	crates/apps/rusty_skillopt/crates/skillopt-model
ts-cli	apps	crates/rusty_tailscale/crates/ts-cli	crates/apps/rusty_tailscale/crates/ts-cli
ts-control	apps	crates/rusty_tailscale/crates/ts-control	crates/apps/rusty_tailscale/crates/ts-control
ts-daemon	apps	crates/rusty_tailscale/crates/ts-daemon	crates/apps/rusty_tailscale/crates/ts-daemon
ts-derp	apps	crates/rusty_tailscale/crates/ts-derp	crates/apps/rusty_tailscale/crates/ts-derp
ts-disco	apps	crates/rusty_tailscale/crates/ts-disco	crates/apps/rusty_tailscale/crates/ts-disco
ts-engine	apps	crates/rusty_tailscale/crates/ts-engine	crates/apps/rusty_tailscale/crates/ts-engine
ts-filter	apps	crates/rusty_tailscale/crates/ts-filter	crates/apps/rusty_tailscale/crates/ts-filter
ts-key	apps	crates/rusty_tailscale/crates/ts-key	crates/apps/rusty_tailscale/crates/ts-key
ts-localapi	apps	crates/rusty_tailscale/crates/ts-localapi	crates/apps/rusty_tailscale/crates/ts-localapi
ts-magicsock	apps	crates/rusty_tailscale/crates/ts-magicsock	crates/apps/rusty_tailscale/crates/ts-magicsock
ts-net	apps	crates/rusty_tailscale/crates/ts-net	crates/apps/rusty_tailscale/crates/ts-net
ts-stun	apps	crates/rusty_tailscale/crates/ts-stun	crates/apps/rusty_tailscale/crates/ts-stun
ts-tun	apps	crates/rusty_tailscale/crates/ts-tun	crates/apps/rusty_tailscale/crates/ts-tun
ts-types	apps	crates/rusty_tailscale/crates/ts-types	crates/apps/rusty_tailscale/crates/ts-types
ts-wg	apps	crates/rusty_tailscale/crates/ts-wg	crates/apps/rusty_tailscale/crates/ts-wg
xtask	apps	crates/rusty_tailscale/xtask	crates/apps/rusty_tailscale/xtask
rusty_text	apps	crates/rusty_text	crates/apps/rusty_text
rusty_voice	apps	crates/rusty_voice	crates/apps/rusty_voice
sessionmgr-agents	apps	crates/rusty_yirp/crates/sessionmgr-agents	crates/apps/rusty_yirp/crates/sessionmgr-agents
sessionmgr-core	apps	crates/rusty_yirp/crates/sessionmgr-core	crates/apps/rusty_yirp/crates/sessionmgr-core
sessionmgr-daemon	apps	crates/rusty_yirp/crates/sessionmgr-daemon	crates/apps/rusty_yirp/crates/sessionmgr-daemon
sessionmgr-desktop	apps	crates/rusty_yirp/crates/sessionmgr-desktop/src-tauri	crates/apps/rusty_yirp/crates/sessionmgr-desktop/src-tauri
sessionmgr-git	apps	crates/rusty_yirp/crates/sessionmgr-git	crates/apps/rusty_yirp/crates/sessionmgr-git
sessionmgr-proc	apps	crates/rusty_yirp/crates/sessionmgr-proc	crates/apps/rusty_yirp/crates/sessionmgr-proc
sessionmgr-protocol	apps	crates/rusty_yirp/crates/sessionmgr-protocol	crates/apps/rusty_yirp/crates/sessionmgr-protocol
sessionmgr-pty	apps	crates/rusty_yirp/crates/sessionmgr-pty	crates/apps/rusty_yirp/crates/sessionmgr-pty
sessionmgr-tui	apps	crates/rusty_yirp/crates/sessionmgr-tui	crates/apps/rusty_yirp/crates/sessionmgr-tui
rusty_boot	tools	crates/rusty_boot	crates/tools/rusty_boot
```

## Appendix C: upward dependency exceptions

Every path dependency edge between members that would point to a higher
layer under Appendix B. The proof script verifies this block against the
graph; it is empty at `cd789d573`.

```layer-exceptions
```

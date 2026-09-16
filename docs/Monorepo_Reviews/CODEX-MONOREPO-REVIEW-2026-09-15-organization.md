# Monorepo organization review — rusty_mill

Reviewed on 2026-09-15 against `main` at `cd789d57314d352bbed6f3eb259a2c0728994510`,
via `/codex-build`. Scope is **repository organization only**: how the 239
workspace members are laid out, named, labelled and mapped, and what a
newcomer can and cannot tell from the tree. It does not reopen any code
finding from rounds 1 to 7 (`CODEX-MONOREPO-REVIEW*.md`) or any duplication
or sovereignty row in `repo-inspector-report.md`. Its recommendations are
consolidated into the proposed
[`docs/adr/0003-workspace-layout-by-layer.md`](docs/adr/0003-workspace-layout-by-layer.md);
this document is the evidence, that one is the decision.

Authorship: the Codex builder could not run from this workstation (see
Limitations), so the Claude host wrote this review and the ADR, and fresh,
separate Claude sessions inspected both (a first round returned REVISE
with 13 findings, all fixed; a second fresh round followed). Same-provider
inspection, recorded as such.

## Method and scope

Commands run in a clean detached worktree at `cd789d573`:

- `cargo metadata --format-version 1 --no-deps --locked` (no build): member
  list, manifest paths, path dependencies, target kinds, `publish`,
  `edition`, `rust-version`, `license`. Every count below derives from it
  unless a file is named instead.
- A stdlib Python pass over that JSON computing per-family membership,
  cross-family edges, external-dependent counts, isolated members, and
  external (non-path) direct dependency counts.
- `grep` over `crates/**/Cargo.toml` for `path = "../..."` versus
  `workspace = true`; over `.github/`, `.config/` and `crates/**/*.md` for
  hard-coded `crates/<name>` paths and cross-family relative links.
- Read in full: root `Cargo.toml`, `ARCHITECTURE.md`, `CONTRIBUTING.md`,
  `docs/adr/0001`, `docs/adr/0002`, `.github/workflows/ci.yml` (jobs and
  the `plan` step), `.github/scripts/affected_crates.py` and
  `cargo_toml_diff.py` headers, `.config/nextest.toml`, and the READMEs of
  `rustils`, `rustils_async`, `rusty_test`, `rusty_libc`, `rusty_win32`,
  `rusty_tokio`, `rusty_serde`, `rusty_meshed`, `rusty_adk`,
  `rusty_provider`, `rusty_search`, plus `crates/nexus/CLAUDE.md`,
  `crates/rusty_boot/{Cargo.toml,src/main.rs}` and
  `crates/rusty_wiremock/src/lib.rs`.
- Skimmed: `README.md` section headings and its crate table (239 rows,
  lines 60 to 308) and the opening of "How the crates relate" (line 336);
  the headers of every prior review round; `docs/atlas/*`.

Severity here means organizational impact, not defect class: **medium** =
a newcomer or a tool draws the wrong conclusion from the tree today;
**low** = inconsistency or drift with a bounded fix.

## Evidence

### E1. Size and shape

| Measure | Value |
|---|---|
| Workspace members | 239 |
| Top-level directories under `crates/` | 82 (all at one depth, no grouping) |
| Members declaring zero external normal dependencies | 72 |
| Members with a `bin` target | 46 across 29 families |
| Member manifests using relative `path = "../..."` deps | 128 (85 of them with at least one that crosses a family boundary) |
| Member manifests inheriting something via `workspace = true` (a dependency or a package field) | 147 |
| Member directories whose last segment differs from the crate name | 21 |

Members with zero external normal dependencies (ADR-0002 Tier-S
candidates; build- and dev-dependencies not examined here), split by
whether their placement says so:

| Placement | Crates |
|---|---|
| Own top-level directory (39) | `rpath`, `rusty_ansder`, `rusty_audio`, `rusty_base64`, `rusty_boot`, `rusty_codec`, `rusty_compress`, `rusty_config`, `rusty_crypto_key`, `rusty_diff`, `rusty_err`, `rusty_font`, `rusty_git`, `rusty_gpu`, `rusty_gui`, `rusty_h2`, `rusty_jinja`, `rusty_kafka`, `rusty_libc`, `rusty_oauth`, `rusty_rag`, `rusty_rand`, `rusty_regx`, `rusty_retry`, `rusty_rsa`, `rusty_rusqlite`, `rusty_sha1`, `rusty_simd`, `rusty_std`, `rusty_stream`, `rusty_sync`, `rusty_text`, `rusty_time`, `rusty_uuid`, `rusty_voice`, `rusty_vulkan`, `rusty_win32`, `rusty_wire`, `rusty_wiremock` |
| Inside a family directory (33), where the family name, not the crate, is what a reader sees | `platform-mock`, `platform-parity`, `winargv`, `coreutils` (rustils); `reactor-core`, `platform-async`, `platform-async-mock`, `threading`, `coreutils-async` (rustils_async); `conformance` (rusty_test); `rusty_serde`, `rusty_serde_derive`, `rusty_serde_erased`; `rusty_json-derive`; `rusty-adk`; `rusty-db`; `rusty-search`; `rusty-meshed-core`, `-domains`, `-governance`, `-registry`, `-sdk`, `-trace`; `rusty-hister-core`, `-crawler`, `-indexer`, `-mcp`, `-server`, `-vectorstore`; `sessionmgr-git`, `sessionmgr-pty`; `nexus-fuzz`; `xtask` |

Zero external dependencies is not the same as being a foundation crate
(`rusty_voice`, `rusty_boot`, the `rusty-meshed-*` binaries are products
built entirely on first-party code), which is why ADR-0003 assigns layers
by purpose and dependency direction, not by this count.

Largest families: `nexus` 42, `rusty_tailscale` 16, `rusty_adk` 14,
`rusty_search` 12, `rusty_meshed` 9, `rusty_yirp` 9, `rusty_agent_gateway`
9, `rustils` 8, `rusty_hister` 8, `rusty_key` 8, `rusty_db` 6,
`rustils_async` 6, `rusty_test` 6, `rusty_provider` 6, `rusty_skillopt` 4;
the other 67 directories hold one to three crates each.

### E2. Who depends on whom across family boundaries

Crates ranked by the number of dependents **outside their own top-level
directory** (path edges of any kind, all targets):

| Dependents | Crate | Lives under |
|---|---|---|
| 19 | `rusty_tokio` | `crates/rusty_tokio` |
| 17 | `rusty_json` | `crates/rusty_json` |
| 16 | `rusty_std` | `crates/rusty_std` |
| 14 | `rusty_wire`, `rusty_request`, `rusty_err` | own directories |
| 13 | `rusty_uuid`, `rusty_http` | own directories |
| **12** | **`platform`** | **`crates/rustils/crates/platform`** |
| 11 | `rusty_base64` | `crates/rusty_base64` |
| 10 | `rusty_win32`, `rusty_sqlite`, `rusty_serde` | own directories |
| **9** | **`platform-linux`** | **`crates/rustils/crates/platform-linux`** |
| 7 | `rusty_time`, `rusty_libc` | own directories |
| 6 | `rusty_regx`, **`rusty-mcp`** | `crates/rusty_regx`, **`crates/rusty_mcp/crates/rusty-mcp`** |
| 5 | `rusty_simd`, `rusty_kafka`, `rpath` | own directories |
| 4 | `rusty_wiremock`, `rusty_tls`, `rusty_retry`, `rusty_rand`, `rusty_gui`, `rusty_font` | own directories |
| 3 | **`platform-windows`**, **`platform-mock`**, `rusty_url`, `rusty_gpu` | `crates/rustils/crates/...`, `crates/rusty_url`, `crates/rusty_gpu` |
| 2 | **`platform-bsd`**, `rusty_a2a`, `rusty_term`, `rusty_sha1`, `rusty_rsa`, `rusty_lines`, `rusty_compress`, `rusty_codec`, `rusty_audio` | |

Bold rows are reusable libraries that live inside another family's
directory. `platform`'s twelve external dependents are `rusty_tokio`,
`rusty_tls`, `rusty_rdp`, `rusty_fedora_agent`, `sessionmgr-pty`,
`nexus-rush`, `ts-magicsock`, `ts-tun`, `platform-async`,
`platform-async-mock`, `platform-async-linux`, `coreutils-async`.

Heaviest cross-family edges (from family to family, counting distinct
crate pairs once regardless of dependency kind or target):
`rusty_meshed`→`rusty_err` 9, `rusty_search`→`rusty_serde` 9,
`rusty_meshed`→`rusty_json` 7, `rusty_search`→`rusty_uuid` 7,
`rustils_async`→`rustils` 6, `rusty_meshed`→`rusty_request` 6,
`rusty_meshed`→`rusty_tokio` 6, `rusty_meshed`→`rusty_http` 5,
`rusty_meshed`→`rusty_kafka` 5, `rusty_meshed`→`rusty_sqlite` 5,
`rusty_search`→`rusty_request` 5, `rusty_tailscale`→`rustils` 4,
`rusty_tailscale`→`rusty_http` 4, `rusty_tls`→`rustils` 4,
`rusty_tokio`→`rustils` 4. Every one of these points from product code to
library code; the graph is already layered, the tree is not.

No crate outside `nexus` depends on a `nexus-*` crate, although
`nexus-plugin-api` and `nexus-hashline` have `[workspace.dependencies]`
entries (root `Cargo.toml`, "Hoisted from nexus" block). No crate outside
`rusty_adk`, `rusty_tailscale`, `rusty_key`, `rusty_yirp`, `rusty_provider`,
`rusty_agent_gateway`, `rusty_meshed`, `rusty_hister`, `rusty_inventrory`,
`rusty_skillopt` or `rusty_test` depends on any of their members.

Isolated members (no workspace dependencies and no workspace dependents):
`nexus-protocol`, `rusty_ansi`, `rusty_config`, `rusty_h2`,
`rusty_rusqlite`, `threading`, `xtask`.

### E3. Metadata spread

| Field | Values across 239 members |
|---|---|
| `edition` | 2021 ×187, 2024 ×52 |
| `rust-version` | unset ×127, 1.75 ×41, 1.88 ×34, 1.85 ×31, 1.82 ×3, 1.70 ×2, 1.86 ×1 |
| `license` | `MIT OR Apache-2.0` ×143, `MIT` ×67, `Apache-2.0` ×27, `UNLICENSED` ×1, unset ×1 |
| `[workspace.package] repository` | `https://github.com/baileyrd/rusty_search` (root `Cargo.toml`, `[workspace.package]` block, a hoisting artifact documented in the comment above it) |

The root `Cargo.toml` comments explain each spread as the residue of
merging former nested workspaces whose `[workspace.package]` values
collided (rusty_db, rustils, rusty_inventrory, rusty_skillopt, rusty_key,
rusty_tailscale, rusty_adk, rusty_provider, rusty_yirp,
rusty_agent_gateway). The explanation is thorough; the values are still
what a tool sees.

## Findings

1. **The OS abstraction layer is filed under a name that reads as one of
   its consumers.** Severity: medium.
   Location: `crates/rustils/crates/platform*`, `crates/rustils/crates/winargv`,
   beside `crates/rustils/crates/coreutils`; `crates/rustils/README.md`
   lines 1 to 6 and 30 to 37.
   Evidence: the README itself is accurate ("a hand-rolled, Rust-native
   platform personality layer for Windows and Linux", line 3; `coreutils`
   is the "reference consumer", line 37). Nothing else says so: the
   directory is named like a coreutils port, `ARCHITECTURE.md`'s
   "Runtime / platform" row names `rustils_async` and `platform-async*`
   but not `platform*`, and E2 shows `platform` (12) and `platform-linux`
   (9) are the most-consumed crates in the workspace without their own
   top-level directory. `rusty_tokio`, `rusty_tls` and `rusty_rdp` all
   reach into `crates/rustils/crates/...` by relative path (`path =
   "../rustils/crates/platform"` and `../rustils/crates/platform-linux`
   each appear 5 times, one of them in the stale nested
   `crates/rustils_async/Cargo.toml`).
   Recommendation: ADR-0003 Phase 1 (move `platform*`/`winargv` to
   `crates/platform/rustils/`, `coreutils` to `crates/apps/coreutils`).

2. **Three OS-abstraction families exist and none is discoverable as one.**
   Severity: medium.
   Location: `crates/rustils/crates/platform*`;
   `crates/rustils_async/crates/{reactor-core,platform-async,platform-async-mock,platform-async-linux,threading}`;
   `crates/rusty_test/crates/{contract,compat,conformance}`.
   Evidence: `crates/rusty_test/README.md` line 1 titles the family
   `portable-runtime-contract` and describes "one execution contract,
   per-host adapters" with `contract/` as "trait boundary only, no
   OS-specific code" and `compat/` as the adapter layer, which is an OS
   abstraction, not a test utility. `crates/rustils_async/README.md`
   lines 1 to 12 describe itself as "a native-async sibling to rustils"
   that depends on rustils' `platform`, `platform-mock`, `platform-linux`.
   `ARCHITECTURE.md`'s "Runtime / platform" row lists `rustils_async`,
   `platform-async*`, `threading` but not `rustils`' own `platform*` and
   not `rusty_test` at all.
   Recommendation: ADR-0003 `crates/platform/` holding all three, with
   `rusty_test` renamed `portable-runtime` to match its own README.

3. **The `crates/` root carries no layer signal.**
   Severity: medium.
   Location: root `Cargo.toml` `[workspace] members` (239 entries, 82
   directory prefixes).
   Evidence: E1. `rusty_std` (16 external dependents, `no_std`) and
   `rusty_homelab_mcp` (a binary with none) are siblings at the same
   depth with the same naming pattern. `ARCHITECTURE.md` says its
   layering table "is illustrative, not exhaustive" and nothing checks it;
   `.github/scripts/check_workspace_deps.py` enforces only that first-party
   crates resolve by path (ADR-0002), not direction.
   Recommendation: ADR-0003 layer directories plus the Phase 0a
   `[package.metadata.rusty_mill] layer` field and direction check, so the
   layout is derived from a checked fact rather than a diagram.

4. **`ARCHITECTURE.md`'s "Application / client" row mixes a library with
   applications.** Severity: low.
   Location: `ARCHITECTURE.md`, Boundaries table, row "Application /
   client": `rusty_proxmox`, `rusty_opnsense`, `rusty_homelab_mcp`,
   `rusty_request`, `rusty_search-*`.
   Evidence: `rusty_request` has 14 dependents in other families (E2) and
   is the HTTP client every homelab client and every `rusty-search-*`
   remote backend builds on; `rusty_homelab_mcp` is a binary with none.
   Recommendation: superseded by ADR-0003's table (`rusty_request` in
   `libs/net/`, the homelab REST clients in `libs/homelab/`, the MCP server
   and agent in `apps/`).

5. **Directory names do not name their crates in 21 places.** Severity: low.
   Location (directory → crate): `crates/rusty_key/crates/{app,compose,config,constrain,feed,kernel,mcp,observe}`
   → `rk-*`; `crates/rusty_provider/crates/{cli,core,mcp,providers,router,server}`
   → `rp-*`; `crates/rusty_acp` → `rusty-acp`; `crates/rusty_croc` →
   `rusty-croc`; `crates/rusty_whisper` → `rusty-whisper`;
   `crates/rusty_db/rusty_db` → `rusty-db`; `crates/rusty_err/derive` →
   `rusty_err_derive`; `crates/rusty_term/l13` → `rusty_term_l13`;
   `crates/rusty_yirp/crates/sessionmgr-desktop/src-tauri` →
   `sessionmgr-desktop`. Plus family prefixes that differ from the
   directory: `rusty_yirp` holds `sessionmgr-*`, `rusty_tailscale` holds
   `ts-*`, `rusty_test` holds `contract`/`compat`/`conformance`.
   Evidence: `cargo metadata` `manifest_path` versus `name`. A reader who
   sees `cargo test -p rk-feed` fail cannot find `rk-feed` with `ls`.
   Recommendation: ADR-0003 naming conventions (single-crate directory
   equals crate name; family READMEs list differing crate prefixes; fix
   `rusty_inventrory` spelling during its move). Renaming crates
   themselves is out of scope except for the unpublished generic names
   below.

6. **Generic, unprefixed crate names sit in a 239-crate namespace.**
   Severity: low.
   Location: `platform`, `platform-{linux,windows,bsd,mock,parity}`,
   `winargv`, `reactor-core`, `platform-async{,-mock,-linux}`, `threading`,
   `coreutils`, `coreutils-async`, `contract`, `compat`, `conformance`,
   `stat-tool`, `proc-runner`, `pty-shell`, `xtask`.
   Evidence: `cargo metadata` reports `publish: []` (that is `publish =
   false`) for all of them except `xtask` (no `publish` key; a
   `rusty_tailscale` build helper). `threading` and `xtask` are also
   isolated (E2). None except `xtask` is a published identity, so a
   rename costs only in-workspace edits.
   Recommendation: ADR-0003 optional Phase 5 for all but `xtask`, which
   is publishable by default and is left out of the rename list;
   `platform` → `rustils-platform` is the one with a clear payoff.

7. **Reusable libraries are co-located with their demos and examples.**
   Severity: low.
   Location: `crates/rusty_mcp/crates/{rusty-mcp,rusty-mcp-demo}`;
   `crates/rusty_adk/examples/{weather-agent,mcp-tool-server,a2a-agent-server}`
   (workspace members).
   Evidence: `rusty-mcp` has 6 external dependents; `rusty-mcp-demo` none.
   Recommendation: keep each family intact but under `libs/` (ADR-0003
   split rule), so the family's placement says "library" even though it
   ships a demo binary.

8. **The shared test double has no README and no visible home.**
   Severity: low.
   Location: `crates/rusty_wiremock/` (contains `Cargo.lock`, `Cargo.toml`,
   `src/` only).
   Evidence: `crates/rusty_wiremock/src/lib.rs` lines 4 to 16 explain it
   replaced four identical `tests/support/` copies in `rusty_proxmox`,
   `rusty_opnsense`, `rusty_fedora`, `rusty_homelab_mcp` (E2: 4
   dependents). The same is true of `crates/rusty_boot/` (no README; its
   purpose is only in `src/main.rs` lines 1 to 3).
   Recommendation: a README for each (out of scope here), and a `libs/`
   placement for `rusty_wiremock`, `tools/` for `rusty_boot` (ADR-0003).

9. **Isolated crates have no stated relationship to anything.** Severity: low.
   Location: `nexus-protocol`, `rusty_ansi`, `rusty_config`, `rusty_h2`,
   `rusty_rusqlite`, `threading`, `xtask` (E2).
   Evidence: zero workspace edges in either direction at this commit.
   `rusty_h2` and `rusty_rusqlite` each have a `docs/adr/` directory, so
   they are deliberate; `rusty_ansi` was reviewed in the PR #65 sweep and
   kept as a different-scoped tool from `rusty_term` (`ARCHITECTURE.md`,
   Key decisions).
   Recommendation: none of these should be deleted on this evidence. Under
   ADR-0003 each has a layer (`rusty_config` foundation, `rusty_h2`
   libs/net, `rusty_rusqlite` libs/storage, `rusty_ansi` libs/ui,
   `threading` platform, `xtask` with its family, `nexus-protocol` with
   nexus), which at least states what they are for.

10. **Overlapping families that a layer directory will place side by side.**
    Severity: low (a note for the map, not a dedupe finding).
    - Storage: `rusty_sqlite` (bundled SQLite, ADR-0002 Tier A),
      `rusty_rusqlite` (isolated), `rusty-db-sqlite` (SQLx-based driver),
      `nexus-database`/`nexus-kv` (product-internal). Under ADR-0003 the
      first three sit in `libs/storage/`; the nexus ones stay with nexus.
    - JSON: `rusty_json` (17 dependents) and `rusty_serde`'s JSON format
      (10 dependents, via `rusty_search`/`rusty_uuid`). Both foundation.
    - HTTP: `rusty_http` (13), `rusty_h2` (0), `rusty_request` (14). All
      `libs/net/`.
    - MCP: `rusty-mcp` (library, 6 dependents) versus `adk-mcp`,
      `agentgateway-mcp`, `rp-mcp`, `rk-mcp`, `nexus-mcp` (each a family's
      own integration crate, none reused). Only `rusty-mcp` is `libs/`.
    - A2A: `rusty_a2a` (library) versus `adk-a2a`, `agentgateway-a2a`
      (integrations). Same pattern.
    Prior dedupe dispositions (`ARCHITECTURE.md`, PR #10 and PR #65;
    `repo-inspector-report.md` Section 1) are not reopened.

11. **Workspace metadata is inconsistent in ways tools notice.**
    Severity: low.
    Location: root `Cargo.toml` `[workspace.package]`; member manifests.
    Evidence: E3. `[workspace.package] repository` points at
    `baileyrd/rusty_search`, so any member inheriting `repository.workspace
    = true` advertises the wrong repo; seven `rust-version` values (and
    127 unset) mean `cargo msrv`-style tooling has no single answer; four
    license values including one `UNLICENSED` and one unset.
    Recommendation: fix `repository` to `https://github.com/Rusty-Mill/rusty_mill`
    (one line); record the MSRV and license policy alongside the Phase 0a
    metadata table (ADR-0003) rather than as a separate sweep.

12. **The dependency map is hand-written and 900 lines long.** Severity: low.
    Location: `README.md` lines 336 to 1222 ("How the crates relate").
    Evidence: the section narrates dependency edges in prose with
    snapshot caveats ("`cargo metadata --all-features` is the authority;
    this list is a snapshot"); `docs/atlas/rusty-mill-atlas-evidence-review.md`
    lesson 10 records that it "already contains stale and internally
    inconsistent relationship claims".
    Recommendation: ADR-0003 Phase 0a generated `docs/WORKSPACE-MAP.md`
    with a staleness check in CI.

13. **A move is cheaper than it looks, provided path deps are hoisted first.**
    Severity: low (a planning fact).
    Evidence: 128 member manifests use relative `path` deps (parsed with
    `tomllib`, all dependency tables including `[target.*]`); 85 of them
    have at least one entry that crosses a family boundary (257 entries),
    and counted at the phase that first breaks each manifest they fall
    9 / 66 / 10 / 0 across ADR-0003's phases 1 to 4. Three stale nested
    `[workspace]` manifests are also tracked (`crates/rustils/Cargo.toml`,
    `crates/rustils_async/Cargo.toml`, `crates/rusty_serde/Cargo.toml`);
    the second still points at `../rustils/crates/platform*`. `Cargo.lock` records path members without a
    `source` (lines 13126 to 13130, 10001 to 10004) and does not change on
    a move. `.github/scripts/affected_crates.py` is path-agnostic
    (manifest paths from `cargo metadata`). Hard-coded paths: `ci.yml`
    lines 162, 420, 427 (`data-mesh-monitor`), comments at 49, 433, 458;
    `setup-build-env/action.yml` line 55; `.config/nextest.toml` none.
    Cross-family relative markdown links under `crates/`: 27 in 11 README
    files (`mill-term`, `rusty_ansder`, `rusty_fedora`,
    `rusty_fedora_agent`, `rusty_git`, `rusty_homelab_mcp`,
    `rusty_meshed`, `rusty_opnsense`, `rusty_proxmox`, `rusty_rag`,
    `rusty_text`); 6 further relative links in
    `crates/rusty_yirp/docs-audit.md` and
    `crates/nexus/docs/adr/0031-cli-scope-exceptions-to-ipc-only.md` are
    already broken today and are not a consequence of any move.
    Recommendation: ADR-0003 Phase 0b before any move.

## Coverage

- Every workspace member's manifest metadata and path edges (via `cargo
  metadata`); every top-level `crates/*` directory is assigned in
  ADR-0003 Appendix B and the proof script confirms the assignment is
  total and direction-clean.
- Root `Cargo.toml` in full; CI workflow, composite actions, plan scripts'
  headers; nextest config; root governance docs; the family READMEs
  named in Method.

## Limitations

- **No Codex pass.** The `codex exec` launch failed in 10 seconds: the
  network's web filter answered `auth.openai.com`'s token refresh with an
  HTTP 401 "WWW Authorization Required" captive page and redirected the
  `chatgpt.com` websocket with 307, so Codex reported "Your access token
  could not be refreshed" and produced nothing. This review and the ADR
  are host-authored (Claude) and inspected by separate fresh Claude
  sessions (two rounds), not by the other provider. The launch artifacts
  are in the session's scratch directory, not in the repo.
- Static inspection only: no `cargo build`, `cargo test` or `cargo clippy`
  was run; nothing here depends on one.
- Member READMEs were read for the families named in Method, not all 82.
  Per-crate `docs/adr/` and `docs/decisions/` series (28 directories at
  any depth under `crates/`) were not read; they may
  contain placement rationale this review did not see.
- The layer assignment of a handful of crates is a judgment call and is
  listed as such in ADR-0003's Consequences.

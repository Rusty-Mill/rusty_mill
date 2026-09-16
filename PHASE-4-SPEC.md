# Work order: ADR-0003 Phase 4 (apps/ and tools/: 21 families, 125 crates — final phase)

## Status: the `git mv` step is already done — read this first

Same environment limitation as Phases 1-3. The host performed every
move directly and committed it (`c8423476e`): 21 family directories
moved wholesale — 20 into `crates/apps/`, 1 (`rusty_boot`) into
`crates/tools/`. `rusty_inventrory` was renamed to `rusty_inventory`
during its move (directory rename only, per the ADR's naming
convention — its crates are already `inventory-*`). `coreutils` and
`coreutils-async` moved from their nested location under
`crates/rustils/crates/` and `crates/rustils_async/crates/` (deferred
there since Phase 1's split); the two now-empty `crates/rustils/` and
`crates/rustils_async/` directory shells were removed, completing the
split Phase 1 started — neither directory exists anymore. `nexus`'s
whole pnpm/shell/packages ecosystem moved as one unit alongside its 42
Cargo crates. Verified: 4786 renames at 100% similarity, zero
deletions. **Do not attempt any `git mv`, file move, or file
deletion.** Your job is only the remaining content edits below.

This is the **last phase** of ADR-0003's migration. Once your edits
land, every workspace member is under its assigned layer directory.

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md` — the
migration plan's Phase 4 row and Appendix B. Not itself independently
re-reviewed (build proceeds under `--unreviewed-spec`); do not edit the
ADR, or `CHANGELOG.md`/`PLAN-REVIEW-LOG.md`/`RELEASE_NOTES.md` — the
host adds Phase 4 entries after your build.

## Ground truth: `PHASE-4-DATA.json`

Every fact below is precomputed in this committed data file — use it as
ground truth, do not re-derive any of it, but do sanity-check and
report if something looks wrong rather than guessing:

- `member_path_mapping` (125 entries): old → new path for every
  workspace member. Use this for Task 1's member-list rewrite.
- `workspace_dependencies_path_fixes` (46 entries): package name → new
  path, for every `[workspace.dependencies]` entry that already exists
  at root and needs its path updated. The other 79 of the 125 members
  have zero external consumers (verified: no `X.workspace = true`
  reference anywhere outside their own family) — do not add a new root
  entry for any of them.
- `exclude_entry_fixes` (4 entries): old → new for every
  `[workspace] exclude` entry needing an update.
- `rush_cascading_fix`: `rush` itself moved this phase (one level
  deeper, `crates/rush/` → `crates/apps/rush/`). Its dependency on
  `rusty_lines` was already fixed once in Phase 3
  (`"../rusty_lines"` → `"../libs/ui/rusty_lines"`, correct for rush's
  *then* location) and needs re-deriving now that rush moved again —
  same cascading-fix class Phase 3 hit with `rusty_oauth`/
  `rusty_request`/`rusty_rag`. The field gives the exact old string
  currently in the file and the exact new string to replace it with.
- `nexus_guard_hardcoded_path_fixes`: 7 Nexus test files under
  `crates/apps/nexus/crates/nexus-bootstrap/tests/` hardcode
  `"crates/nexus/crates"` and/or `"crates/nexus/shell"` as a literal
  path-join argument (not a fixed-depth `ancestors()` walk — all 7
  already use the robust walk-up-to-`[workspace]` pattern from Phase 1's
  `layering.rs` fix, verified directly; only the hardcoded prefix
  *strings* need updating). For each file, replace every occurrence of
  the `old` string with `new` (some entries specify a `count` — that
  many occurrences in that file; replace all of them identically).
  **Do not touch `ipc_topic_prefix_invariant.rs`'s separate
  `"crates/nexus-bootstrap/src"` reference** (line ~135 in the pre-move
  file) — that string was never a valid path even before this
  migration (it's missing the `crates/nexus/crates/` prefix entirely),
  a pre-existing unrelated bug, not something this migration touches or
  should silently "fix" as a side effect.
- `ci_yml_functional_fixes`: 3 lines in `.github/workflows/ci.yml`
  (the `data-mesh-monitor` job's change-filter, `working-directory`,
  and `cache-dependency-path`) that are real, executable CI
  configuration — `rusty_meshed`'s vendored dashboard moved this phase.
- `comment_only_fixes`: 3 files (root `Cargo.toml`, `ci.yml`,
  `.github/actions/setup-build-env/action.yml`) each with one doc
  comment citing an old `crates/nexus/...` path for a human reader —
  update the path string only, nothing else on the line.

## Task 1 — root `Cargo.toml`

Apply `member_path_mapping` to `[workspace] members` (125 entries),
`workspace_dependencies_path_fixes` to `[workspace.dependencies]` (46
entries — change only the `path` value on each, leave every other key
untouched), and `exclude_entry_fixes` to `[workspace] exclude` (4
entries). Apply the one `Cargo.toml` entry from `comment_only_fixes`.
Touch nothing else in this file.

## Task 2 — `.github/workflows/ci.yml` and `.github/actions/setup-build-env/action.yml`

Apply `ci_yml_functional_fixes` (3 lines, real CI behavior — get these
exactly right, a typo here breaks the `data-mesh-monitor` job for every
future PR touching `rusty_meshed`) and the two `comment_only_fixes`
entries for these files. Touch nothing else.

## Task 3 — `crates/apps/rush/Cargo.toml`

Apply `rush_cascading_fix`: change only the `rusty_lines` dependency's
`path` value from `"../libs/ui/rusty_lines"` to
`"../../libs/ui/rusty_lines"`. Every other key on that line (there's a
`version` and `default-features`) stays exactly as it is.

## Task 4 — the 7 Nexus test guards

Apply every fix in `nexus_guard_hardcoded_path_fixes`. These are string
literals passed to `.join(...)` or assigned to a `const` — a plain text
replacement of the old string for the new one, nothing about the
surrounding logic changes. Verify after: `cargo test -p nexus-bootstrap
--test dep_invariants --test plugin_contract_purity --test
tauri_command_boundary --test bootstrap_coverage --test
core_plugin_loc_budget --test ipc_topic_prefix_invariant --test
dep_invariants_shell` should compile and run (pass or fail on their own
merits, but they must at minimum *find* the nexus crates at the new
path rather than reporting an empty member list or a missing directory
— re-read each test's own assertions if any fail, since a few of these
guards actively assert real architectural invariants about nexus's
crate graph, not just path resolution).

## Task 5 — `README.md`

Update both occurrences (link target + backtick path) on each of the
125 crate-table rows for the moved families to their new
`crates/apps/...` or `crates/tools/...` path per
`member_path_mapping`. Touch nothing else.

## Non-goals

- No `git mv` / directory moves (already done).
- Do not touch `ipc_topic_prefix_invariant.rs`'s pre-existing unrelated
  `"crates/nexus-bootstrap/src"` bug (see Task 4 above).
- Do not touch any manifest, CI file, or dependency entry outside
  Tasks 1-5.
- Do not add a `[package.metadata.rusty_mill]` table anywhere.
- `nexus`'s own `pnpm-workspace.yaml`/`package.json`/`.github/` (its
  pre-merge CI, dead since the merge per the root `ci.yml`'s own header
  comment) need no edit — verified they don't reference their own
  absolute repo-relative path anywhere; they're self-contained relative
  to their own directory, which moved as a unit.

## Acceptance criteria

- The moves are already committed — your own `git status` should show
  only: root `Cargo.toml`, `README.md`, `.github/workflows/ci.yml`,
  `.github/actions/setup-build-env/action.yml`,
  `crates/apps/rush/Cargo.toml`, and the 7 Nexus test guard files
  modified. Nothing else. If you see anything beyond those 11 files
  changed, stop and report.
- `cargo metadata --format-version=1 --all-features --locked` succeeds.
- **Dependency-graph identity** by package name (not resolve-node id):
  compare against the host's pre-move snapshot at
  `C:\tmp\phase4-metadata.json`. Every package's resolved deps and
  features must be identical.
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  still passes (64 tests; this phase touches no script).
- `check_workspace_deps.py`, `check_workspace_layers.py`, and
  `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` all exit
  0 — regenerate and commit the map if it's stale (it shouldn't be,
  since `package_family()` is already correct for every directory
  shape this phase introduces, but verify rather than assume).
- All 7 Nexus guard tests listed in Task 4 compile and run.
- A repo-wide grep for the literal strings `"crates/coreutils"`,
  `"crates/coreutils-async"`, `"crates/mill-term/"`, `"crates/nexus/"`,
  `"crates/rush/"`, `"crates/rusty_agent_gateway/"`,
  `"crates/rusty_croc/"`, `"crates/rusty_fedora_agent/"`,
  `"crates/rusty_hister/"`, `"crates/rusty_homelab_mcp/"`,
  `"crates/rusty_inventrory/"`, `"crates/rusty_key/"`,
  `"crates/rusty_meshed/"`, `"crates/rusty_multimodal_db/"`,
  `"crates/rusty_provider/"`, `"crates/rusty_skillopt/"`,
  `"crates/rusty_tailscale/"`, `"crates/rusty_text/"`,
  `"crates/rusty_voice/"`, `"crates/rusty_yirp/"`, and
  `"crates/rusty_boot/"` returns no hits outside: `docs/adr/0003-
  workspace-layout-by-layer.md`, `CHANGELOG.md`, `PLAN-REVIEW-LOG.md`,
  `RELEASE_NOTES.md`, `CODEX-MONOREPO-REVIEW*.md`, `docs/atlas/*`,
  `repo-inspector-report.md`, `docs/adr/000[12]*`, any per-crate
  `docs/adr/*`/`docs/decisions/ADR-*` file, `PHASE-*-SPEC.md`,
  `crates/rusty_multimodal_db/docs/design/*.md` (prose, not links), and
  `ipc_topic_prefix_invariant.rs`'s one pre-existing unrelated bug (see
  Task 4). Anything else is something this spec missed — fix it or
  report it.
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in earlier phases; use per-package
  batches as supplemental proof.
- Delete `PHASE-4-DATA.json` before finishing — it's a build-time
  working artifact, not something to ship.

## Known cost, not a defect

`affected_crates.py` will mark all 125 moved crates plus their
dependents as affected — the largest CI matrix in this migration,
essentially a full-workspace sweep given how much of the workspace
ultimately depends on `nexus`, `rush`, or one of the other moved
families.

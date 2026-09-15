# Work order: ADR-0003 Phase 0b (hoist cross-family path deps)

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md`, migration
table row "0b" and the paragraph beginning "Phase 0b is what makes
phases 1 to 4 cheap". That ADR is independently Codex-approved (see
`PLAN-REVIEW-LOG.md`). This document is a host-derived, narrower
restatement scoped to exactly what to build; not itself independently
re-reviewed (build proceeds under `--unreviewed-spec`).

## Ground truth: `PHASE-0B-DATA.json`

The host already computed the exact set of in-scope entries against this
branch's base commit (`aa9e32b6a`) and committed it alongside this spec:
**247 entries across 85 member manifests, targeting 53 distinct crates**
(13 of which already have a `[workspace.dependencies]` entry at root; 40
need a new one). Use this file as ground truth — do not recompute the
cross-family set from scratch and do not second-guess which entries are
in scope. Each entry has: `member` (crate name), `manifest_path`
(repo-root-relative), `section` (`dependencies` /
`dev-dependencies` / `build-dependencies`, or a
`target.<cfg>.<section>` variant), `dep_key` (the key in that TOML
table), `target_crate` (the depended-on crate's package name),
`member_path_value` (the literal `path = "..."` string currently in that
manifest), `root_relative_path` (the same target resolved to a
repo-root-relative path — use this for any new root
`[workspace.dependencies]` entry), and `extra_keys` (every other key
already on that dependency spec, e.g. `features`, `optional`,
`default-features`, `version`).

As a sanity check only (not to override the data file): if you re-derive
"cross-family" independently and get a materially different manifest
count (85 ± a handful), stop and report the discrepancy — don't silently
prefer your own recomputation over the committed data file, and don't
silently prefer the data file if your own check turns up something it
clearly missed. Either way, surface it.

`distinct_target_crates`, `targets_already_hoisted`, and
`targets_needing_new_entry` in the same JSON file give you the crate-name
lists for the two steps below without re-deriving them from `entries`.

## Task — hoist each cross-family entry

**Step 1 — root `Cargo.toml`'s `[workspace.dependencies]`.**

For the 13 crates in `targets_already_hoisted`: nothing to add here. Just
sanity-check the existing entry's `path` matches the `root_relative_path`
your entries for that target crate expect (it should — stop and report
if not, that would mean two different things share a name, a
pre-existing bug worth surfacing rather than working around).

For the 40 crates in `targets_needing_new_entry`: add one new entry each,
`name = { path = "<root_relative_path>" }`. Look at every entry in
`entries` for that `target_crate`: if *all* of them that have a
`version` in `extra_keys` agree on the exact same version string, add
`version = "..."` to the new root entry too (version belongs to the
target crate, not to each consumer). If they disagree, stop and report
it. Insert each new entry into the existing `[workspace.dependencies]`
table near the other `path`-based entries already there (see the
`rusty-search-*`/`rusty-db-*` block for the established style) — don't
reorder or reformat any existing entry, and don't create a duplicate key
for anything already in `targets_already_hoisted`.

**Step 2 — the 247 member-manifest entries.**

For each entry in `entries`, edit the one line/table it names (by
`manifest_path` + `dep_key` + `section`) so that dependency reads
`{ workspace = true, ...<remaining extra_keys> }` instead of using
`path = "..."`, where `<remaining extra_keys>` is `extra_keys` **minus**
`version` *only if* you just hoisted that same version to the target's
new root entry in Step 1 (or the target was already in
`targets_already_hoisted` and its existing root entry already carries
that same version) — otherwise, if the root entry has no `version`, keep
the member's own `version` key as a local override. Concretely:

- `name = { path = "...", features = ["x"] }` → `name = { workspace = true, features = ["x"] }`
- `name = { path = "..." }` with an otherwise-empty `extra_keys` →
  `name.workspace = true` (or `name = { workspace = true }` — match
  whichever style that specific manifest already uses elsewhere for its
  other `workspace = true` deps, if any; otherwise either is fine)
- A block-table form (`[dependencies.name]` on its own line, `path =
  "..."` on the next) stays a block table; only the key changes.
- If `extra_keys` still has a `package` key after removing `version`
  (none of the 247 entries do, per the data file, but check): keep it —
  `alias = { workspace = true, package = "real-name" }`.
- Touch nothing else in these files: comments, unrelated dependencies,
  formatting, blank lines all stay exactly as they are.

## Non-goals

- Any *same-family* relative path dependency (not in `PHASE-0B-DATA.json`)
  stays untouched — it's correct as-is until its family actually moves
  in Phases 1-4.
- No `git mv` / directory moves (still not this phase).
- No touching `[package.metadata.rusty_mill]` tables (Phase 0a, already
  merged), CI scripts, or any crate's actual Rust source.

## Acceptance criteria

- `cargo metadata --format-version=1 --all-features --locked` still
  succeeds.
- **Dependency-graph identity.** Capture `cargo metadata
  --format-version=1 --all-features --locked` before your changes
  (already captured by the host at `C:\tmp\phase0b-metadata.json` against
  this exact base commit — reuse it as the "before" snapshot rather than
  regenerating) and after. Every `resolve.nodes[].id`'s resolved `deps`
  (which package id each edge points to) and `features` list must be
  identical — this PR is a pure manifest reorganization with zero effect
  on what actually builds. Also do the same comparison with plain `cargo
  metadata --format-version=1 --locked` (no `--all-features`) — that's
  the more sensitive check for the handful of entries carrying
  `default-features`/`optional`, where `--all-features` alone could mask
  a real behavior change.
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  still passes unmodified (this phase touches no script).
- `check_workspace_deps.py` and `check_workspace_layers.py` against
  fresh metadata still exit 0.
- `git diff --stat` shows: root `Cargo.toml` (new/reused
  `[workspace.dependencies]` entries only) + exactly the 85 manifests
  named in `PHASE-0B-DATA.json` + no other file (delete
  `PHASE-0B-DATA.json` itself before finishing — it's a build-time
  working artifact, not something to ship; the host will note its
  contents in `PLAN-REVIEW-LOG.md` instead).
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in `PLAN-REVIEW-LOG.md`'s Phase 0a
  inspection (unrelated to this change, doesn't reproduce on
  `ubuntu-latest` CI); use per-package batches as supplemental proof.

## Known cost, not a defect

Only the 85 touched manifests (plus their dependents) are "affected" by
`affected_crates.py`'s change-detection this time — expect a partial,
not full, CI matrix.

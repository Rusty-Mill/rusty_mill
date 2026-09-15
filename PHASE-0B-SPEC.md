# Work order: ADR-0003 Phase 0b (hoist cross-family path deps)

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md`, migration
table row "0b" and the paragraph beginning "Phase 0b is what makes
phases 1 to 4 cheap". That ADR is independently Codex-approved (see
`PLAN-REVIEW-LOG.md`). This document is a host-derived, narrower
restatement scoped to exactly what to build; not itself independently
re-reviewed (build proceeds under `--unreviewed-spec`).

## Ground truth: `PHASE-0B-DATA.json` (v2 — supersedes the round-1 attempt)

A first build attempt against this same spec (minus this section) hit a
real Cargo restriction: `default-features = false` cannot be set on a
`workspace = true` dependency unless the workspace-level entry itself
also disables default features, and the workspace worktree was reset
back to the clean prep baseline (`e1c8520cc`) before this v2 data file
was generated — do not assume anything on disk reflects that first
attempt. The host resolved this by excluding the 6 affected entries from
the hoist entirely (see `excluded_entries` below) rather than changing
any root entry's `default-features`, because every *other* consumer of
those 5 target crates needs defaults on, and downgrading the shared root
default to satisfy one minority consumer would be a real behavior change
for everyone else, not a safe mechanical hoist.

The host computed the exact set of in-scope entries against this
branch's base commit (`e1c8520cc`) and committed it alongside this spec:
**241 entries to hoist, across 83 member manifests, targeting 52
distinct crates** (12 of which already have a `[workspace.dependencies]`
entry at root; 40 need a new one) — plus **6 entries in `excluded_entries`
that must NOT be hoisted**, each with its own `excluded_reason`. Use this
file as ground truth — do not recompute the cross-family set from
scratch, do not second-guess which entries are in scope, and do not
attempt to "fix" the 6 excluded entries by any means (changing a root
entry's `default-features`, wrapping in a feature flag, etc.) — leave
each of those 6 dependency lines completely untouched, still using
`path = "..."`, exactly as it is today. Fields on each entry: `member`
(crate name), `manifest_path` (repo-root-relative), `section`
(`dependencies` / `dev-dependencies` / `build-dependencies`, or a
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
count (83 ± a handful, or you find *another* `default-features = false`
conflict this data file doesn't already list in `excluded_entries`), stop
and report the discrepancy — don't silently prefer your own
recomputation over the committed data file, and don't silently prefer
the data file if your own check turns up something it clearly missed.
Either way, surface it; don't invent a fix for a newly-found conflict
the same way you correctly didn't invent one for the first six.

`distinct_target_crates`, `targets_already_hoisted`,
`targets_needing_new_entry`, and `manifests_touched` in the same JSON
file give you the exact name lists for the steps below without
re-deriving them from `entries`.

## Task — hoist each cross-family entry

**Step 1 — root `Cargo.toml`'s `[workspace.dependencies]`.**

For the 12 crates in `targets_already_hoisted`: nothing to add here. Just
sanity-check the existing entry's `path` matches the `root_relative_path`
your entries for that target crate expect (it should — stop and report
if not, that would mean two different things share a name, a
pre-existing bug worth surfacing rather than working around). Do not add
or change a `default-features` key on any existing entry, including the
5 targets that have an excluded entry (`rusty_wire`, `rusty_json`,
`rusty_simd`, `rusty_serde` are pre-existing hoisted or need-new targets
whose sole `default-features = false` consumer is excluded — leave their
root entry exactly as this data file's `targets_already_hoisted` /
`targets_needing_new_entry` lists already account for; `rusty_lines`
likewise gets a plain new entry with no `default-features` key, since its
one remaining consumer, `rusty_boot`, doesn't override it).

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

**Step 2 — the 241 member-manifest entries in `entries` (not `excluded_entries`).**

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
  (none of the 241 entries do, per the data file, but check): keep it —
  `alias = { workspace = true, package = "real-name" }`.
- Touch nothing else in these files: comments, unrelated dependencies,
  formatting, blank lines all stay exactly as they are. This includes the
  6 `excluded_entries` lines — leave them as direct `path` dependencies,
  unchanged, even in a manifest you're otherwise editing for its other
  entries (e.g. `rush`, `rusty_oauth`, `rusty_request`, `rusty_uuid` each
  have one excluded entry alongside other entries that do hoist — edit
  only the hoisting ones).

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
  `[workspace.dependencies]` entries only) + exactly the 83 manifests
  listed in `PHASE-0B-DATA.json`'s `manifests_touched` + no other file
  (delete `PHASE-0B-DATA.json` itself before finishing — it's a
  build-time working artifact, not something to ship; the host will note
  its contents, including the 6 exclusions, in `PLAN-REVIEW-LOG.md`
  instead).
- `git diff --numstat` per touched manifest shows added lines equal to
  deleted lines (a pure `path = "..."` → `workspace = true` swap changes
  the same line(s), not a net addition) — a manifest showing only
  additions or only deletions in this diff would mean something besides
  the intended swap happened.
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in `PLAN-REVIEW-LOG.md`'s Phase 0a
  inspection (unrelated to this change, doesn't reproduce on
  `ubuntu-latest` CI); use per-package batches as supplemental proof.

## Known cost, not a defect

Only the 83 touched manifests (plus their dependents) are "affected" by
`affected_crates.py`'s change-detection this time — expect a partial,
not full, CI matrix.

# Work order: ADR-0003 Phase 0a (metadata + checker + generated map)

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md`, section
"Phase 0a in detail" (and Appendix A `layer-order`, Appendix B
`layer-map`, Appendix C `layer-exceptions`). That ADR is independently
Codex-approved (see `PLAN-REVIEW-LOG.md`, round 4, plan SHA256
`227e9a404f7c13e235fdb3754cfa13c2764446d53d89442e11f18fbf9e3a7247`).
This document is a narrower, host-derived restatement of that section
scoped to exactly what to build in this PR; it has not itself been
independently re-reviewed (build proceeds under `--unreviewed-spec`).
It adds no requirement the ADR doesn't already state — read the ADR
section above for full rationale; this file exists to pin down the
mechanical details (file names, CLI shapes, exact wiring points) so
nothing is invented mid-build.

## Scope — read carefully

**In scope (Phase 0a only):**
1. A `[package.metadata.rusty_mill]` table with a `layer` key added to
   every one of the 239 workspace member `Cargo.toml` files.
2. A new CI checker script + unit tests.
3. A generated `docs/WORKSPACE-MAP.md` + a way to verify it isn't stale
   + CI wiring for that.
4. `README.md`'s hand-maintained crate-relationship narrative replaced
   with a short pointer to the generated map.
5. `.github/workflows/ci.yml` updated to run the new checker and the
   map-staleness check.

**Out of scope — do not do any of this:**
- No `git mv` / no directory moves of any kind. Every crate stays at its
  current path. (Phases 1-4 do the moves; not this PR.)
- No hoisting of `path = "../..."` deps into `[workspace.dependencies]`
  (that's Phase 0b).
- No `tier` field (the ADR calls it optional; Appendix B has no tier
  column and deriving ADR-0002 tier classifications for 239 crates is a
  separate, unscoped exercise — omit it entirely this PR).
- No edits to any crate's actual source code, dependencies, or
  `[package]` fields other than adding the new `[package.metadata.rusty_mill]`
  table.
- No renames (Phase 5).

## Task 1 — per-crate layer metadata

For every workspace member, add to its `Cargo.toml`:

```toml
[package.metadata.rusty_mill]
layer = "foundation"   # or platform | libs | apps | tools
```

The correct `layer` value per crate is given by the `layer-map` block in
`docs/adr/0003-workspace-layout-by-layer.md` (Appendix B) — tab-separated
`crate-name`, `layer`, `current-directory`, `proposed-directory`. Match on
`crate-name` (== the package's `name` in `cargo metadata`). Place the new
table immediately after the existing `[package]` table in each manifest
(before `[dependencies]`), preserving every other line in the file
byte-for-byte otherwise. If a manifest already has a
`[package.metadata.*]` table for something else, add `rusty_mill` as a
sibling table, do not merge into or disturb the existing one.

Do this for all 239 members listed in Appendix B — no more, no fewer. If
`cargo metadata --format-version=1 --all-features --locked` (run from a
clean tree before starting) reports a workspace member whose name is
*not* in Appendix B, or Appendix B names a crate that is not a current
workspace member, stop and report the mismatch instead of guessing a
layer for it — Appendix B is supposed to be a total, current mapping,
so a mismatch means something changed since the ADR was written and
needs a human decision, not an invented layer.

## Task 2 — CI checker

New file `.github/scripts/check_workspace_layers.py`, following the
existing sibling `.github/scripts/check_workspace_deps.py`'s conventions
exactly: stdlib-only, a module docstring explaining what/why, one or more
pure functions taking already-parsed `cargo metadata` JSON (so
`test_check_workspace_layers.py` can exercise them against synthetic
metadata with no `cargo` invocation, matching
`.github/scripts/test_check_workspace_deps.py`'s style), a thin `main()`
that reads the metadata path from `sys.argv[1]`, prints one violation per
line to stderr with a clear message, and `sys.exit(1)` if any violation
was found (exit 0 otherwise, printing nothing on success).

Three checks, all against `cargo metadata --format-version=1
--all-features` output for the *current* (unmoved) workspace:

1. **Missing layer.** Every workspace member must have
   `package.metadata.rusty_mill.layer` set to one of `foundation`,
   `platform`, `libs`, `apps`, `tools`. Flag any member missing the table,
   the key, or with an unrecognized value.
2. **Upward edges.** For every path/workspace dependency edge between two
   workspace members (any dependency kind — normal, dev, build; use the
   resolve graph's node/deps adjacency, restricted to both ends being
   workspace members), the depended-on crate's layer must be the same as
   or earlier than the depending crate's layer in the order `foundation,
   platform, libs, apps, tools` (that literal order — parse it from the
   ADR's `layer-order` fenced block in Appendix A, or hard-code the same
   five names in that order with a comment pointing at Appendix A; either
   is fine, but the order must come from that appendix, not be
   reinvented). Flag any edge that points to a strictly later layer.
3. **Cross-family apps edges.** Restricted to edges whose *depending*
   crate's layer is `apps`: the depended-on crate must not be an `apps`
   crate in a different family. "Family" = the path component
   immediately following `crates/` in the depending/depended-on crate's
   manifest directory (e.g. `crates/rusty_meshed/crates/rusty-meshed-core`
   → family `rusty_meshed`; `crates/rusty_boot` → family `rusty_boot`).
   A depending crate whose own layer is `tools` is exempt from this
   check entirely (the `tools` layer may depend on everything, including
   apps crates in other families — `crates/rusty_boot/Cargo.toml`'s
   `rush = { path = "../rush" }` is the existing edge this exemption
   covers; include it as an accepted case in the test file, not a
   violation).

Run it locally against the real, now-annotated workspace
(`cargo metadata --format-version=1 --all-features > /tmp/metadata.json
&& python3 .github/scripts/check_workspace_layers.py /tmp/metadata.json`)
and confirm it exits 0 with no output — the ADR's own review already
established the current dependency graph has zero upward edges under
this exact layer assignment (`CODEX-MONOREPO-REVIEW-2026-09-15-organization.md`,
E1/E2, and Appendix C's `layer-exceptions` block is empty), so a
violation here means either the checker or the Task 1 metadata has a
bug — fix it, don't add an exception.

`.github/scripts/test_check_workspace_layers.py`: unit tests against
synthetic metadata (no `cargo` needed), same style as
`test_check_workspace_deps.py` — cover: a clean workspace with no
violations; a member missing the layer key; a member with an
unrecognized layer value; an edge from `foundation` to `platform`
(upward, flagged); an edge from `platform` to `foundation` (downward,
clean); a same-family `apps`→`apps` edge (clean, family-internal edges
are unconstrained per the ADR); a cross-family `apps`→`apps` edge
(flagged); a cross-family `tools`→`apps` edge matching the
`rusty_boot`→`rush` shape (clean, exempted).

## Task 3 — generated workspace map

New file `.github/scripts/generate_workspace_map.py` (stdlib-only, same
family of scripts). Two modes:
- Default: read `cargo metadata` JSON from `sys.argv[1]`, print the
  generated Markdown for `docs/WORKSPACE-MAP.md` to stdout.
- `--verify PATH`: generate the same content and compare it byte-for-byte
  against the file at `PATH`; exit 0 if identical, exit 1 with a clear
  message ("docs/WORKSPACE-MAP.md is stale — regenerate with ...") if
  not, printing nothing further (no diff dump needed, the message is
  enough).

Table columns: layer, family, crate name, one-line description (from
that package's `[package] description` in `cargo metadata`; if the
manifest has no description, use an empty cell, not a placeholder
string), dependents count. Dependents count = the total number of other
workspace members with a path/workspace dependency (any kind) on this
crate, from the same resolve-graph adjacency Task 2 uses — this is a
simple total across the whole workspace, not filtered to
cross-family-only (the ADR doesn't specify cross-family-only for this
column; that framing was specific to the review's own E2 evidence
table, not to this generated file). Sort rows by layer (in Appendix-A
order), then family, then crate name.

Generate the file, commit it as `docs/WORKSPACE-MAP.md`, and verify with
`python3 .github/scripts/generate_workspace_map.py /tmp/metadata.json
--verify docs/WORKSPACE-MAP.md` (exit 0).

`.github/scripts/test_generate_workspace_map.py`: unit tests against
synthetic metadata covering at least: correct column values for a small
synthetic workspace, missing-description handling, sort order, and that
`--verify` against a matching file passes while `--verify` against a
mismatched file fails.

## Task 4 — README.md

Replace the hand-maintained "How the crates relate" section (currently
lines 336 to 1222 at `main`'s HEAD before this PR — re-locate by heading
text, not by line number, since line numbers will have shifted by the
time this runs) with a short paragraph explaining that the crate
relationship map is now generated at `docs/WORKSPACE-MAP.md` (linked),
regenerated by `.github/scripts/generate_workspace_map.py`, and checked
for staleness in CI. Keep the crate table above it (the 239-row table
with `crates/...` links) and the "History" section as-is — only the
narrative dependency prose is replaced.

## Task 5 — CI wiring

In `.github/workflows/ci.yml`, in the existing `dependency-policy` job
(it already runs unconditionally on every PR and already produces
`/tmp/metadata.json`), add two steps after the existing
`check_workspace_deps.py` step:

```yaml
      - run: python3 .github/scripts/check_workspace_layers.py /tmp/metadata.json
      - run: python3 .github/scripts/generate_workspace_map.py /tmp/metadata.json --verify docs/WORKSPACE-MAP.md
```

(Adjust exact flag/arg spelling to match whatever CLI Task 2/3's scripts
actually implement — keep them consistent with each other and with what
you just wrote.) No new job — `test_check_workspace_layers.py` and
`test_generate_workspace_map.py` are auto-discovered by the existing
`plan-tests` job's `unittest discover -p 'test_*.py'`, no workflow change
needed there.

## Acceptance criteria

- All 239 workspace members carry a correct `[package.metadata.rusty_mill]
  layer` matching Appendix B, and no other manifest content changed.
- `cargo metadata --format-version=1 --all-features --locked` still
  succeeds (manifests remain valid TOML/Cargo).
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  passes, including the two new test files.
- `check_workspace_layers.py` exits 0 against the real workspace's
  metadata (zero violations).
- `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` exits 0
  against the real workspace's metadata.
- `cargo fmt --all -- --check` still passes (no `.rs` files touched by
  this PR, so this should be a no-op confirmation).
- No `git status` renames/moves — only modified/added files, no deletions
  outside the README narrative-section replacement.
- `PLAN-REVIEW-LOG.md` already has a "Build — Phase 0a" entry recording
  this work order; do not edit that file as part of the build.

## Known cost, not a defect

Because every one of the 239 crates' own `Cargo.toml` changes, `.github/
scripts/affected_crates.py` will mark all 239 as "affected" on this PR,
so the full CI matrix (build/test/clippy/cross-compile across the whole
workspace) will run, not just `fmt`/`dependency-policy`/`plan-tests`.
This is expected for this PR given its shape (every manifest touched)
and is not something to engineer around — do not add anything to
`affected_crates.py` or `cargo_toml_diff.py` to suppress it; that's out
of scope and would weaken a real safety net.

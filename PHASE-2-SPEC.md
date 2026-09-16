# Work order: ADR-0003 Phase 2 (foundation/: 26 families, 30 crates)

## Status: the `git mv` step is already done — read this first

Same environment limitation as Phase 1: Codex's sandbox cannot write
`.git/worktrees/<name>/index.lock` (outside a linked worktree's own
directory tree). The host performed every move directly and committed
it (`30afc0c9e`): 26 family directories moved wholesale from
`crates/<name>` to `crates/foundation/<name>` (each carries its own
nested sub-crates automatically — `rusty_err`'s `derive/`, `rusty_json`'s
`rusty_json-derive/`, `rusty_serde`'s three sub-crates), and the third
stale pre-merge nested workspace manifest,
`crates/rusty_serde/Cargo.toml`, deleted (`rustils`'s and
`rustils_async`'s were deleted in Phase 1). Verified: 350 renames at
100% similarity, 1 deletion, nothing else. **Do not attempt any `git
mv`, file move, or file deletion.** Your job is only the remaining
content edits below.

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md` — the
migration plan's Phase 2 row and Appendix B (`layer-map`) for the
canonical crate list. Not itself independently re-reviewed (build
proceeds under `--unreviewed-spec`); do not edit the ADR itself, or
`CHANGELOG.md`/`PLAN-REVIEW-LOG.md`/`RELEASE_NOTES.md` (dated log
entries, historical) — the host adds Phase 2 entries to those after your
build.

## The 30 moved crates (26 families)

```
rpath, rusty_ansder, rusty_base64, rusty_codec, rusty_compress,
rusty_config, rusty_crypto_key, rusty_diff, rusty_err, rusty_err_derive,
rusty_jinja, rusty_json, rusty_json-derive, rusty_libc, rusty_rand,
rusty_regx, rusty_retry, rusty_rsa, rusty_serde, rusty_serde_derive,
rusty_serde_erased, rusty_sha1, rusty_simd, rusty_std, rusty_sync,
rusty_time, rusty_url, rusty_uuid, rusty_win32, rusty_wire
```

Every one of these now lives at `crates/foundation/<its family's old
top-level name>/...` — e.g. `rusty_err_derive` moved from
`crates/rusty_err/derive` to `crates/foundation/rusty_err/derive`;
`rusty_serde_erased` moved from `crates/rusty_serde/rusty_serde_erased`
to `crates/foundation/rusty_serde/rusty_serde_erased`.

## Task 1 — root `Cargo.toml`

**`[workspace] members`** — rewrite the path for all 30 entries above to
prepend `foundation/` after `crates/` (e.g.
`"crates/rusty_std"` → `"crates/foundation/rusty_std"`,
`"crates/rusty_err/derive"` → `"crates/foundation/rusty_err/derive"`,
`"crates/rusty_serde/rusty_serde"` →
`"crates/foundation/rusty_serde/rusty_serde"`). All 30 are still exactly
where the workspace member list already has them today, just under the
new prefix — don't add, remove, or reorder any entry.

**`[workspace.dependencies]`** — rewrite the `path` value (nothing else
on the line) for exactly these 24 entries, which already exist at root
(verified against the current file; the other 6 of the 30 —
`rusty_ansder`, `rusty_config`, `rusty_err_derive`, `rusty_json-derive`,
`rusty_serde_derive`, `rusty_serde_erased` — have zero consumers outside
their own family or, for the two `derive`/`erased` helpers, only a
same-family sibling reference via a literal relative path that survives
the move unedited (see Task 2) — none of the 6 needs a new root entry,
don't add one):

```
rpath, rusty_base64, rusty_codec, rusty_compress, rusty_crypto_key,
rusty_diff, rusty_err, rusty_jinja, rusty_json, rusty_libc, rusty_rand,
rusty_regx, rusty_retry, rusty_rsa, rusty_serde, rusty_sha1, rusty_simd,
rusty_std, rusty_sync, rusty_time, rusty_url, rusty_uuid, rusty_win32,
rusty_wire
```

Each becomes `path = "crates/foundation/<name>"` (or
`"crates/foundation/rusty_serde/rusty_serde"` for the one entry whose
current path is already nested one level deeper) matching the move
table above. Leave every other key on each line (e.g. `version`,
`features`) untouched.

**`[workspace] exclude`** — change the single entry
`"crates/rusty_libc/bench"` to `"crates/foundation/rusty_libc/bench"`
(the bench directory moves with its family, same rule as Phase 1's
`fuzz` entry).

## Task 2 — 3 manifests outside `foundation/` with a literal path into it

Phase 0b excluded 6 dependency entries from its hoist because Cargo
rejects `default-features = false` on a `workspace = true` dependency
unless the workspace-level entry also disables default features (see
`PLAN-REVIEW-LOG.md`'s Phase 0b entries for the full reasoning — the
decision was to leave each as a direct `path` dependency rather than
change a shared root default for one minority consumer). Of those 6,
3 involve a target crate that just moved in this phase while the
*consumer* stays where it is (it's a `libs`-layer crate, not moving
until Phase 3) — those 3 relative paths now point at a directory that no
longer exists there and need a manual fix (the other 3 excluded entries
resolved for free: both endpoints moved together this phase, so their
relative path is unchanged — nothing to do for those, don't touch them):

```
crates/rusty_oauth/Cargo.toml:    rusty_json  path "../rusty_json"   -> "../foundation/rusty_json"
crates/rusty_request/Cargo.toml:  rusty_json  path "../rusty_json"   -> "../foundation/rusty_json"
crates/rusty_rag/Cargo.toml:      rusty_simd  path "../rusty_simd"   -> "../foundation/rusty_simd"
```

Change only the `path` value on that one line in each of the 3 files;
every other key on that dependency entry (`default-features`, `features`,
etc.) stays exactly as it is. Do not touch anything else in these three
manifests.

## Task 3 — `README.md`

The crate table has one row per workspace member with the directory
path appearing twice (link target + literal backtick path), same format
as Phase 1. Update both occurrences on each of the 30 rows for the
crates listed above to their new `crates/foundation/...` path. Touch
nothing else on those lines and no other row.

## Non-goals

- No `git mv` / directory moves (already done, see Status above).
- No Rust source changes — verified by grep across every `.rs` file in
  the repo: **zero** files hardcode a `crates/<name>/` path for any of
  these 26 families (unlike Phase 1's `layering.rs`, nothing in this
  phase's move breaks a Rust-level path assumption).
- Do not touch any manifest or dependency entry outside the lists in
  Task 1 and Task 2.
- Do not add a `[package.metadata.rusty_mill]` table anywhere — already
  present on every member since Phase 0a.

## Acceptance criteria

- The moves are already committed — your own `git status` should show
  only: root `Cargo.toml`, `README.md`, and the 3 manifests in Task 2
  modified. Nothing else. If you see anything beyond those 5 files
  changed, stop and report.
- `cargo metadata --format-version=1 --all-features --locked` succeeds.
- **Dependency-graph identity**: compare `cargo metadata
  --format-version=1 --all-features --locked` before (host's snapshot at
  `C:\tmp\phase2-metadata.json`, captured against this exact base commit
  — reuse it) and after, **by package name** (not resolve-node id — a
  node id embeds the manifest path, which every one of these 30 crates'
  necessarily changed; comparing by name is the correct invariant, same
  as Phase 1's inspection). Every package's resolved deps and features
  must be identical.
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  still passes unmodified (this phase touches no script).
- `check_workspace_deps.py` and `check_workspace_layers.py` against
  fresh metadata still exit 0.
- **`generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` must also
  exit 0** — Phase 1's first CI run failed exactly this check (a since-
  fixed bug in `package_family()` made the generated Family column wrong
  for every crate under a layer directory). Regenerate
  `docs/WORKSPACE-MAP.md` with `python3 .github/scripts/
  generate_workspace_map.py <metadata.json> > docs/WORKSPACE-MAP.md` and
  commit the result — do not skip this the way the spec's first draft
  did last phase.
- A repo-wide grep for the literal string `"crates/<name>/"` for each of
  the 26 family names (excluding the family's own now-current directory,
  i.e. `crates/foundation/<name>/`) returns no hits outside:
  `docs/adr/0003-workspace-layout-by-layer.md`, `CHANGELOG.md`,
  `PLAN-REVIEW-LOG.md`, `RELEASE_NOTES.md`, `CODEX-MONOREPO-REVIEW*.md`,
  `docs/atlas/*`, `repo-inspector-report.md`, `docs/adr/0001*`, and this
  spec file itself (all historical/plan documents, verified by the host
  already). Anything else is something this spec missed.
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in earlier phases (unrelated, doesn't
  reproduce on `ubuntu-latest` CI); use per-package batches as
  supplemental proof.

## Known cost, not a defect

`affected_crates.py` will mark all 30 moved crates plus their
dependents as affected. The root `Cargo.toml` path edits are not a pure
`[workspace.members]` addition, so `cargo_toml_diff.py` will likely
classify this as unsafe and force the full CI matrix, same as Phase 1 —
expected, not something to engineer around.

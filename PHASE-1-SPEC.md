# Work order: ADR-0003 Phase 1 (platform/: rustils split, rustils_async split, rusty_test → portable-runtime)

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md` — the
family split rule table, the migration plan's Phase 1 row, "What each
move phase touches besides `git mv`", and Appendix B (`layer-map`) for
every crate's exact proposed path. That ADR is independently
Codex-approved (see `PLAN-REVIEW-LOG.md`). This document is a
host-derived, narrower restatement, with every fact below verified
directly against the current repository state (not just read off the
ADR) before writing this spec — do not re-derive them, but do sanity
check anything that looks off and stop and report rather than guess.
Not itself independently re-reviewed (build proceeds under
`--unreviewed-spec`).

**Do not edit `docs/adr/0003-workspace-layout-by-layer.md` itself** — it
is the approved plan; Appendix B's "current directory" column stays as
the historical snapshot it was approved as, even after this PR moves
those directories. Same for `CHANGELOG.md` and `PLAN-REVIEW-LOG.md`'s
existing entries — those are dated log entries describing past state,
not living documents; the host will add this phase's own new entries to
each after your build, don't add to them yourself.

## What moves (18 crates + their family's shared files)

**`rustils` split** — 7 crates move to `crates/platform/rustils/crates/`,
`coreutils` (an 8th crate in the same current directory) does **not**
move this phase (it's `apps`-layer, deferred to Phase 4):

```
crates/rustils/crates/platform         -> crates/platform/rustils/crates/platform
crates/rustils/crates/platform-bsd     -> crates/platform/rustils/crates/platform-bsd
crates/rustils/crates/platform-linux   -> crates/platform/rustils/crates/platform-linux
crates/rustils/crates/platform-mock    -> crates/platform/rustils/crates/platform-mock
crates/rustils/crates/platform-parity  -> crates/platform/rustils/crates/platform-parity
crates/rustils/crates/platform-windows -> crates/platform/rustils/crates/platform-windows
crates/rustils/crates/winargv          -> crates/platform/rustils/crates/winargv
```

Plus every other top-level entry currently under `crates/rustils/`
**except** `crates/coreutils/` and `Cargo.toml`:
`.github/`, `.gitignore`, `CHANGELOG.md`, `LICENSE`, `README.md`,
`deny.toml`, `docs/`, `fuzz/`, `rustfmt.toml` — these move to
`crates/platform/rustils/` (same names, same content, untouched) because
the ADR's family-split rule says the governing docs "stay with the
platform crates, which is what they govern." `crates/rustils/Cargo.toml`
is **deleted, not moved** — it's a stale pre-merge nested `[workspace]`
manifest (verified: its own `members` list is just the 7 platform crates
+ `coreutils`, its `[workspace.dependencies]` duplicates entries the
real root already has). After this phase, `crates/rustils/` will contain
only `crates/coreutils/` until Phase 4 finishes the split — that's the
correct, expected intermediate state, not a bug to "fix" by also moving
`coreutils` early (out of scope this phase, see Non-goals).

**`rustils_async` split** — 5 crates move to
`crates/platform/rustils_async/crates/`, `coreutils-async` stays behind
(same reasoning as `coreutils`):

```
crates/rustils_async/crates/platform-async       -> crates/platform/rustils_async/crates/platform-async
crates/rustils_async/crates/platform-async-linux -> crates/platform/rustils_async/crates/platform-async-linux
crates/rustils_async/crates/platform-async-mock  -> crates/platform/rustils_async/crates/platform-async-mock
crates/rustils_async/crates/reactor-core         -> crates/platform/rustils_async/crates/reactor-core
crates/rustils_async/crates/threading            -> crates/platform/rustils_async/crates/threading
```

Plus every other top-level entry under `crates/rustils_async/` **except**
`crates/coreutils-async/` and `Cargo.toml`: `.github/`, `.gitignore`,
`ARCHITECTURE.md`, `CHANGELOG.md`, `CODE_OF_CONDUCT.md`,
`CONTRIBUTING.md`, `LICENSE`, `README.md`, `RELEASE_NOTES.md`,
`SECURITY.md`, `docs/`, `gap-analysis.md`, `rustfmt.toml` — move to
`crates/platform/rustils_async/` (same names, untouched).
`crates/rustils_async/Cargo.toml` is **deleted, not moved** (same stale
nested-`[workspace]` situation as `rustils`, verified the same way).

**`rusty_test` → `portable-runtime`** — the whole family moves intact
(not split), including a directory rename:

```
crates/rusty_test/crates/compat          -> crates/platform/portable-runtime/crates/compat
crates/rusty_test/crates/conformance     -> crates/platform/portable-runtime/crates/conformance
crates/rusty_test/crates/contract        -> crates/platform/portable-runtime/crates/contract
crates/rusty_test/tools/proc-runner      -> crates/platform/portable-runtime/tools/proc-runner
crates/rusty_test/tools/pty-shell        -> crates/platform/portable-runtime/tools/pty-shell
crates/rusty_test/tools/stat-tool        -> crates/platform/portable-runtime/tools/stat-tool
```

Plus `.github/`, `.gitignore`, `ARCHITECTURE.md`, `CONTRACT.md`,
`README.md` -> `crates/platform/portable-runtime/` (same names,
untouched). `crates/rusty_test/` has no nested `Cargo.toml` to delete —
verified, it never had one. After this move `crates/rusty_test/` no
longer exists at all (nothing stays behind).

Use `git mv` for every one of the above (directories and individual
files alike), not copy+delete, so history stays reachable via `git log
--follow` and the diff renders as renames.

## Root `Cargo.toml`

**`[workspace] members`** — rewrite the path for exactly these 18
entries to their new location (verified: these are the only 18 member
paths matching the crate names above):

```
crates/rustils/crates/platform, crates/rustils/crates/platform-bsd,
crates/rustils/crates/platform-linux, crates/rustils/crates/platform-mock,
crates/rustils/crates/platform-parity, crates/rustils/crates/platform-windows,
crates/rustils/crates/winargv, crates/rustils_async/crates/platform-async,
crates/rustils_async/crates/platform-async-linux,
crates/rustils_async/crates/platform-async-mock,
crates/rustils_async/crates/reactor-core, crates/rustils_async/crates/threading,
crates/rusty_test/crates/compat, crates/rusty_test/crates/conformance,
crates/rusty_test/crates/contract, crates/rusty_test/tools/proc-runner,
crates/rusty_test/tools/pty-shell, crates/rusty_test/tools/stat-tool
```

Do **not** change the member-list entries for `crates/rustils/crates/
coreutils` or `crates/rustils_async/crates/coreutils-async` — those stay
exactly where they are this phase.

**`[workspace.dependencies]`** — rewrite the `path` value (nothing else
on the line) for exactly these 12 entries, which already exist at root
(verified against the current file — every other Phase-1 crate has zero
consumers anywhere and correctly has no root entry; do not add one):

```
platform, platform-bsd, platform-linux, platform-mock, platform-parity,
platform-windows, winargv, platform-async, platform-async-linux,
compat, conformance, contract
```

Each becomes `path = "crates/platform/<rustils|rustils_async|
portable-runtime>/crates/<name>"` matching the move table above. Leave
every other key on each of those lines (e.g. any `version`) untouched.
Do not touch the `coreutils`/`coreutils-async` entries (they aren't in
`[workspace.dependencies]` at all — confirmed, nothing outside their own
family depends on them). Do not add entries for `platform-async-mock`,
`reactor-core`, `threading`, `proc-runner`, `pty-shell`, or `stat-tool` —
verified none of these has any consumer anywhere in the workspace except
same-family siblings using their own relative `path = "../..."` (which
move correctly as a unit and need no edit) or the two stale nested
manifests you're deleting anyway.

**`[workspace] exclude`** — change the single entry `"crates/rustils/
fuzz"` to `"crates/platform/rustils/fuzz"` (the fuzz directory moves with
the platform crates per the ADR's "fuzz and bench directories move with
their family" rule).

## Rust source: `layering.rs`

`crates/rusty_test/crates/conformance/tests/layering.rs` (after the
move: `crates/platform/portable-runtime/crates/conformance/tests/
layering.rs`) has two hardcoded assumptions that only held before this
move:

1. `const GROUP_PREFIX: &str = "crates/rusty_test/";` (line 78) →
   change to `"crates/platform/portable-runtime/"`.
2. `fn workspace_root() -> PathBuf` (line 80) currently does
   `Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(4)` — a fixed
   depth that was correct at 4 levels up before the move but is wrong
   after (the new path is one level deeper: `crates/platform/
   portable-runtime/crates/conformance` vs. today's `crates/rusty_test/
   crates/conformance`). Replace the whole function body with the
   dynamic walk-up-to-the-nearest-`[workspace]`-manifest pattern already
   used by `crates/nexus/crates/nexus-bootstrap/tests/dep_invariants.rs`'s
   own `workspace_root()` (copy that exact logic — loop upward from
   `CARGO_MANIFEST_DIR`, at each level check whether `Cargo.toml` exists
   and contains the literal string `"[workspace]"`, return that
   directory, panic if you walk off the root with a clear message). This
   is more robust than a fixed depth and won't need touching again if a
   future ADR phase changes nesting further.

No other Rust source file in the repository hardcodes a `rustils`,
`rustils_async`, or `rusty_test` path — verified by grep across every
`.rs` file; if your own sweep after the move finds something this spec
missed, fix it the same way (derive dynamically, don't hardcode the new
path either) and report what you found.

## `README.md`

The crate table has one row per workspace member, each with the crate's
directory path appearing twice — once as a Markdown link target, once as
a literal backtick-quoted path, e.g.:

```
| [`platform`](crates/rustils/crates/platform) | `crates/rustils/crates/platform` | ... |
```

Update both occurrences on each of the 18 rows for the crates listed
above to their new path. Touch nothing else on those lines (the
crate-name link text and the description column stay exactly as they
are) and touch no other row. `ARCHITECTURE.md` and `CONTRIBUTING.md`
need no change — verified neither currently mentions any of these three
family paths.

## Non-goals

- Do **not** move `crates/rustils/crates/coreutils` or `crates/
  rustils_async/crates/coreutils-async` — deferred to Phase 4 along with
  every other `apps`/`tools` crate, by design (see the ADR's own phase
  table: Phase 1 moves 18 members, not 20).
- Do not touch `crates/rusty_serde/Cargo.toml` (the third stale nested
  workspace manifest) — that's Phase 2's job, a different family.
- Do not fix the Nexus test guards (`bootstrap_coverage.rs`,
  `dep_invariants.rs`, `plugin_contract_purity.rs`,
  `tauri_command_boundary.rs`, `core_plugin_loc_budget.rs`,
  `dep_invariants_shell.rs`, `ipc_topic_prefix_invariant.rs`) — those
  hardcode `nexus` paths, not `rustils`/`rusty_test`, and `nexus` doesn't
  move until Phase 4.
- Do not add `[package.metadata.rusty_mill]` tables — already present on
  every member since Phase 0a, and moving a file doesn't change its
  content.
- Do not touch any `[workspace.dependencies]` entry or member manifest
  outside the lists above.

## Acceptance criteria

- `git status` after your changes shows the 18 crate directories plus
  every family-level file above as renames (`R`, ideally high similarity
  — content is unchanged, only the path moved), the 2 stale nested
  `Cargo.toml` files as deletions, and exactly the file edits named above
  (root `Cargo.toml`, `layering.rs`, `README.md`) — nothing else.
- `git log --follow -- crates/platform/rustils/crates/platform/Cargo.toml`
  (and spot-check one or two more) still shows history from before the
  move.
- `cargo metadata --format-version=1 --all-features --locked` succeeds
  from the new layout.
- **Dependency-graph identity**: diff `cargo metadata --format-version=1
  --all-features --locked` before (capture it yourself before starting —
  the host's own pre-move snapshot is at `C:\tmp\phase1-metadata.json`,
  reuse it) against after. Every node's resolved `deps` and `features`
  must be identical — a directory move changes `manifest_path`, not the
  dependency graph, so this must come back with zero differences, same
  as Phase 0a and 0b.
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  still passes (untouched by this phase).
- `check_workspace_deps.py` and `check_workspace_layers.py` against
  fresh metadata still exit 0 — `check_workspace_layers.py` in
  particular re-validates every layer/family assignment against the
  *actual* manifest directories, so this is a strong signal the move
  landed the crates in the layer this ADR says they belong in.
- `cargo test -p conformance --test layering` (or whatever the actual
  package/test-binary name is once resolved) passes from the new
  location — this is the one test whose own logic you changed, so it
  needs to actually run, not just compile.
- A repo-wide grep for the literal strings `"crates/rustils/"`,
  `"crates/rustils_async/"`, and `"crates/rusty_test/"` across every
  tracked file returns **no hits outside**: `docs/adr/0003-workspace-
  layout-by-layer.md` (unedited, historical/plan document), `CHANGELOG.md`
  and `PLAN-REVIEW-LOG.md` (unedited by you, dated log entries), and any
  file under `docs/atlas/`, `CODEX-MONOREPO-REVIEW*.md`,
  `repo-inspector-report.md`, or `RELEASE_NOTES.md` (all explicitly
  historical per the ADR's own "Historical documents are not rewritten"
  rule). Anything else that turns up is something this spec missed —
  fix it or report it, don't leave a dangling reference.
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in earlier phases' inspections
  (unrelated, doesn't reproduce on `ubuntu-latest` CI); use per-package
  batches as supplemental proof.

## Known cost, not a defect

`affected_crates.py` will mark the 18 moved crates plus every one of
their dependents as affected — likely a large but not full-workspace CI
matrix (narrower than Phase 0a's, since only these families and their
consumers are touched, not every crate's own manifest).

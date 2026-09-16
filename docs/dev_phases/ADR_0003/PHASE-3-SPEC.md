# Work order: ADR-0003 Phase 3 (libs/: 34 families, 66 crates)

## Status: the `git mv` step is already done — read this first

Same environment limitation as Phases 1-2: Codex's sandbox cannot write
`.git/worktrees/<name>/index.lock` (outside a linked worktree's own
directory). The host performed every move directly and committed it
(`726e8398f`): 34 family directories moved wholesale — 31 into one of 7
thematic subdirectories under `crates/libs/` (`ai`, `async`, `homelab`,
`net`, `protocol`, `storage`, `ui`, per the ADR's "libs/ subdirectories"
table), 3 directly under `crates/libs/` with no theme (`rusty_adk`,
`rusty_git`, `rusty_wiremock`). Verified: 1666 renames at 100%
similarity, no deletions. **Do not attempt any `git mv`, file move, or
file deletion.** Also already fixed, in a separate prior commit
(`a08d010fa`): `check_workspace_layers.py`'s `package_family()` now
skips a `libs/` theme directory as well as the layer directory — do not
touch that function again unless you find it's still wrong.

One incidental side effect of the move, already investigated and
accepted, not something to fix: 8 markdown files (4 in
`rusty_rusqlite`, 4 in `rusty_gui`) had pre-existing CRLF line endings
that got normalized to LF when staged (matching this repo's own
`.gitattributes` LF policy) — their content is unchanged, only
`git log --follow` loses the thread at the move commit for those 8
specific files. Leave this alone.

Source of truth: `docs/adr/0003-workspace-layout-by-layer.md` — the
migration plan's Phase 3 row, the "libs/ subdirectories" table, and
Appendix B for the canonical crate list. Not itself independently
re-reviewed (build proceeds under `--unreviewed-spec`); do not edit the
ADR, or `CHANGELOG.md`/`PLAN-REVIEW-LOG.md`/`RELEASE_NOTES.md` (dated
log entries) — the host adds Phase 3 entries after your build.

## The 34 families and their new location

```
ai/:       rusty_llama, rusty_rag, rusty_whisper
async/:    rusty_stream, rusty_tokio (carries rusty_tokio-macros)
homelab/:  rusty_fedora, rusty_opnsense, rusty_proxmox
net/:      rusty_h2, rusty_http, rusty_kafka, rusty_oauth, rusty_rdp,
           rusty_request, rusty_tls
protocol/: rusty_a2a, rusty_acp, rusty_lsp,
           rusty_mcp (carries rusty-mcp, rusty-mcp-demo)
storage/:  rusty_db (6 crates), rusty_rusqlite,
           rusty_search (12 crates), rusty_sqlite
ui/:       rusty_ansi, rusty_audio, rusty_font, rusty_gpu, rusty_gui,
           rusty_lines, rusty_term (carries rusty_term_l13 as `l13`),
           rusty_vulkan
(no theme): rusty_adk (14 crates), rusty_git, rusty_wiremock
```

Every family is now at `crates/libs/<theme>/<family>/...` (or
`crates/libs/<family>/...` for the 3 with no theme) — e.g.
`crates/rusty_tls` → `crates/libs/net/rusty_tls`;
`crates/rusty_search/crates/rusty-search-core` →
`crates/libs/storage/rusty_search/crates/rusty-search-core`.

## Task 1 — root `Cargo.toml`

**`[workspace] members`** — rewrite the path for all 66 entries to their
new `crates/libs/...` location per the mapping above (e.g.
`"crates/rusty_search/crates/rusty-search-core"` →
`"crates/libs/storage/rusty_search/crates/rusty-search-core"`). All 66
are already exactly where the member list has them today; only the
prefix changes.

**`[workspace.dependencies]`** — rewrite the `path` value (nothing else
on the line) for exactly these 20 entries, which already exist at root
(verified against the current file; every other Phase-3 family has zero
external consumers or only same-theme/same-family consumers whose
relative path survives unedited — see Task 2 — so none of them needs a
new root entry):

```
rusty_tokio, rusty_fedora, rusty_opnsense, rusty_proxmox, rusty_http,
rusty_kafka, rusty_request, rusty_tls, rusty_a2a, rusty_lsp,
rusty_sqlite, rusty_audio, rusty_font, rusty_gpu, rusty_gui,
rusty_lines, rusty_term, rusty_vulkan, rusty_git, rusty_wiremock
```

Each becomes `path = "crates/libs/<theme-or-none>/<name>"` matching the
table above. Leave every other key on each line untouched.

**`[workspace] exclude`** — update these 4 entries to their new
location (fuzz/bench directories move with their family, same rule as
earlier phases):

```
"crates/rusty_term/fuzz"  -> "crates/libs/ui/rusty_term/fuzz"
"crates/rusty_lines/bench" -> "crates/libs/ui/rusty_lines/bench"
"crates/rusty_tls/fuzz"   -> "crates/libs/net/rusty_tls/fuzz"
"crates/rusty_lsp/fuzz"   -> "crates/libs/protocol/rusty_lsp/fuzz"
```

## Task 2 — the 1 manifest outside `libs/` with a literal path into it

Same situation as Phase 2's 3 fixes: Phase 0b excluded 6 dependency
entries from its hoist over a Cargo `default-features` restriction (see
`PLAN-REVIEW-LOG.md`'s Phase 0b entry). 5 of the 6 have already been
closed out — 2 resolved for free in Phase 2 because both endpoints moved
together that phase, 3 fixed by hand in Phase 2. This phase closes the
6th and last one: `rush` (an `apps`-layer crate, not moving until
Phase 4) depends on `rusty_lines` (moving now) via a literal path that
breaks:

```
crates/rush/Cargo.toml:  rusty_lines  path "../rusty_lines"  ->  "../libs/ui/rusty_lines"
```

Change only the `path` value on that one line; every other key on that
entry (there may be a `version` and/or `default-features` key) stays
exactly as it is.

A comprehensive sweep of every remaining literal `path` dependency
targeting any Phase-3 crate found 16 other entries besides this one —
all 16 resolve for free because both the consumer and the target are
either the same family (moves together) or both moving to the *same*
new theme directory this phase, so their relative path is unchanged
(e.g. `rusty_gpu`→`rusty_gui`, `rusty_term`→`rusty_font`,
`rusty_request`→`rusty_tls`, all within `ui/` or `net/`;
`rusty-search-sqlite-fts5`→`rusty_sqlite`, both landing in `storage/`).
**Do not touch any of those 16** — verify a couple by hand if you like,
but they need no edit.

## Task 3 — 3 comment-only path references

Caught by this phase's own acceptance-criterion grep (see Task 4). None
of these are functional code or config — each is a doc comment or `#`
comment pointing a human reader at a path for further reading. Fix each
by updating only the path string, nothing else on the line:

```
Cargo.toml (near the exclude list, 2 occurrences):
  "crates/rusty_lines/bench is a standalone..." -> "crates/libs/ui/rusty_lines/bench is a standalone..."
  "cd crates/rusty_lines/bench && cargo run" -> "cd crates/libs/ui/rusty_lines/bench && cargo run"

crates/platform/portable-runtime/tools/pty-shell/src/main.rs (doc comment):
  `crates/rusty_term/src/render.rs` -> `crates/libs/ui/rusty_term/src/render.rs`

crates/rusty_key/crates/feed/Cargo.toml (comment):
  see crates/rusty_sqlite/Cargo.toml -> see crates/libs/storage/rusty_sqlite/Cargo.toml
```

## Task 4 — `README.md`

Update both occurrences (link target + backtick path) on each of the 66
crate-table rows for the families above to their new
`crates/libs/...` path. Touch nothing else.

## Non-goals

- No `git mv` / directory moves (already done).
- Do not touch `docs/adr/0002-dependency-sovereignty-policy.md` or any
  per-crate `docs/adr/*`/`docs/decisions/ADR-*` file (e.g.
  `crates/rusty_hister/docs/decisions/ADR-0002-*.md`) — these cite
  `crates/<name>` paths as history/evidence, same exemption as ADR-0001
  and ADR-0003 itself.
- Do not touch `crates/rusty_multimodal_db/docs/design/*.md` — its
  mentions of `crates/rusty_http`/`crates/rusty_tls` are prose in
  backtick-code, not Markdown links (the ADR's own rule: prose mentions
  need no edit, only actual broken links do).
- Do not add a `[package.metadata.rusty_mill]` table anywhere.
- Do not touch any manifest or dependency entry outside Tasks 1-3.

## Acceptance criteria

- The moves are already committed — your own `git status` should show
  only: root `Cargo.toml`, `README.md`, `crates/rush/Cargo.toml`,
  `crates/platform/portable-runtime/tools/pty-shell/src/main.rs`, and
  `crates/rusty_key/crates/feed/Cargo.toml` modified. Nothing else. If
  you see anything beyond those 5 files changed, stop and report.
- `cargo metadata --format-version=1 --all-features --locked` succeeds.
- **Dependency-graph identity** by package name (not resolve-node id —
  same reasoning as Phases 1-2): compare against the host's pre-move
  snapshot at `C:\tmp\phase3-metadata.json`. Every package's resolved
  deps and features must be identical.
- `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v`
  still passes (64 tests; this phase touches no script beyond the
  already-committed `package_family()` fix).
- `check_workspace_deps.py`, `check_workspace_layers.py`, and
  `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` all exit 0
  — regenerate and commit the map, same requirement as Phase 2.
- A repo-wide grep for the literal string `"crates/<name>/"` for each of
  the 34 family names (excluding each family's own new directory under
  `crates/libs/`) returns no hits outside: `docs/adr/0003-workspace-
  layout-by-layer.md`, `CHANGELOG.md`, `PLAN-REVIEW-LOG.md`,
  `RELEASE_NOTES.md`, `CODEX-MONOREPO-REVIEW*.md`, `docs/atlas/*`,
  `repo-inspector-report.md`, `docs/adr/0001*`, `docs/adr/0002*`, any
  per-crate `docs/adr/*` or `docs/decisions/ADR-*` file, any
  `PHASE-*-SPEC.md`, and `crates/rusty_multimodal_db/docs/design/*.md`
  (prose, not links — verified by the host). Anything else is something
  this spec missed.
- `cargo fmt --all -- --check` is expected to hit the same pre-existing
  Windows argv-length limit noted in earlier phases; use per-package
  batches as supplemental proof.

## Known cost, not a defect

`affected_crates.py` will mark all 66 moved crates plus their dependents
as affected — a large partial CI matrix, likely the biggest yet given
how central `rusty_tokio`/`rusty_http`/`rusty_search` etc. are to the
rest of the workspace.

# Plan review log — ADR-0003 workspace layout

Roles: host=claude (planner), reviewer=codex, builder=none (review only, no
implementation authorized this session).

Plan: `docs/adr/0003-workspace-layout-by-layer.md`
Repo: `C:\dev\rusty_mill`
Mode: review (existing plan, no requirements interview)
Rounds: max 5

## Round 1 — 2026-09-15

- Provider: codex (codex-cli 0.154.0), model: CLI default (unresolved)
- Session id: 01a0a645-761d-7713-8e5d-bbf3baf3c283
- Plan SHA256: 78f2f24e95cbc7a1e0e13c1073c96c0264403626568bd8a90c1693902fab4fad
- Verdict: **REVISE**
- Summary: The plan matches the supplied SHA256, but its move inventory
  omits executable path consumers and its checker rule rejects an
  explicitly permitted existing dependency.

### Findings

**R1 (high)** — Move-phase inventory misses Rust test consumers of hardcoded
crate-group paths.
- `crates/rusty_test/crates/conformance/tests/layering.rs:78-89` —
  `GROUP_PREFIX = "crates/rusty_test/"` and `workspace_root()` uses
  `ancestors().nth(4)`; both break once `rusty_test` moves under a layer
  directory. Verified against the file — confirmed.
- `crates/nexus/crates/nexus-bootstrap/tests/bootstrap_coverage.rs:102-125`
  — `NEXUS_CRATES_PREFIX = "crates/nexus/crates/"` filters workspace
  members; a move would make the filter match zero members and trip the
  `assert!(!members.is_empty())` guard. Verified against the file (actual
  path is `crates/nexus/crates/nexus-bootstrap/...`, not
  `nexus-bootstrap/...` as cited) — confirmed, citation path was
  abbreviated but the code and line range are correct.
- Codex also flagged (not independently verified this round): other Nexus
  guards with old executable paths — `dep_invariants.rs`,
  `plugin_contract_purity.rs`, `tauri_command_boundary.rs`.
- Fix proposed: add these Rust test consumers to the corresponding move
  PRs; derive family paths from manifest locations instead of a fixed
  ancestor depth; update member filtering while keeping the nonempty
  assertions; expand the executable-path inventory beyond workflow files.

**R2 (high)** — Proposed CI checker rejects an existing, plan-permitted
dependency.
- `crates/rusty_boot/Cargo.toml:31` — `rush = { path = "../rush" }`.
  Verified against the file — confirmed.
- Per Appendix B, `rusty_boot` is assigned to `tools` and `rush` to `apps`
  — different families. Phase 0a's checker rule ("reject any edge to an
  apps crate in another family") would fail immediately on this edge, even
  though the plan's tools-family row says it should be allowed and
  explicitly justifies this integration-harness dependency.
- Fix proposed: restrict the cross-family apps prohibition to callers that
  are themselves in `apps`, while keeping the general upward-edge check;
  explicitly permit tools→apps dependencies and add this edge to the
  checker's acceptance cases.

### Coverage
Verified plan SHA256; inspected root workspace membership/exclusions/
dependency declarations, `check_workspace_deps.py`, `affected_crates.py`;
searched Rust/shell/PowerShell/Python sources for executable path
references and manifest-directory assumptions; traced root member-list
readers in conformance and Nexus bootstrap guards beyond the plan's file
inventory; inspected representative split-family manifests, excluded
fuzz/bench manifests, and `rusty_boot`'s dependency on `rush`.

### Limitations (reviewer-reported)
Static review only, no files edited/tests run/work delegated; did not
regenerate `cargo metadata` or verify every dependency edge/migration
count/doc link; git unavailable on reviewer's PATH so HEAD was not
verified against `cd789d573`; no checker implementation exists yet in the
plan, so enforcement findings concern its specified behavior only.

### Host disposition
User chose: accept both findings, revise plan, re-review. Edited the ADR:
(1) added a "What each move phase touches besides `git mv`" bullet naming
the Rust test files that hardcode crate-group paths and specifying the
root-finding and prefix-literal fixes; (2) reworded the Phase 0a checker
item 2 to restrict the cross-family apps prohibition to `apps`-layer
callers, exempting `tools` (documented `rusty_boot`'s `rush` dependency
as the worked example). Also updated the ADR's "Objections recorded
during drafting" section to record this review round and point at this
log. Sent for round 2.

## Round 2 — 2026-09-15

- Session id: 01a0a645-761d-7713-8e5d-bbf3baf3c283 (resumed)
- Plan SHA256: 368f6d9157b03f9318f464ec30d8b7a7ba73618f05403009bc82b2c7934410e9
- Verdict: **REVISE**
- Summary: R2 resolved. R1 partially resolved — three more Nexus guards
  under `crates/nexus/crates/nexus-bootstrap/tests/` hardcode the same
  family-path literal and were missing from the round-1 fix:
  `core_plugin_loc_budget.rs`, `dep_invariants_shell.rs`,
  `ipc_topic_prefix_invariant.rs`.

### Host disposition
Verified all three by reading the files directly — confirmed hardcoded
`"crates/nexus/crates"` / `"crates/nexus/shell"` literals in each. Ran
`rg -F '"crates/nexus' --type rust crates/nexus` to check the five-file
list was exhaustive; it returned exactly seven files (the four already
listed plus these three), matching Codex's finding with no additional
misses. Rewrote the move-phase-touches bullet to name all seven Nexus
files plus `layering.rs`, and to instruct a repeat of the same `rg`
sweep at move time rather than presenting any fixed file list as
authoritative for a family this ADR did not already grep. Sent for
round 3.

## Round 3 — 2026-09-15

- Session id: 01a0a645-761d-7713-8e5d-bbf3baf3c283 (resumed)
- Plan SHA256: 03786b690b5265f146ea1c78678498fd168d992d1e1029523a32f2b5c7124d6d
- Verdict: **APPROVED**
- Summary: R1 closed at the plan level — all seven Nexus guards and
  `layering.rs` covered, with fixes required in their move phases and a
  repeat search for remaining consumers. R2 remains closed. No findings.

### Coverage (round 3)
Verified SHA256; compared revised migration instructions against both
prior reviews; repeated the Nexus Rust literal search and confirmed the
seven affected files are listed; rechecked `layering.rs`'s family prefix
and fixed-depth workspace-root lookup against the prescribed fix.

### Limitations (round 3, reviewer-reported)
Approval covers the proposed plan; implementation remains unverified. No
files edited, tests run or work delegated. This follow-up focused on
prior-finding closure; dependency counts and the complete graph were not
independently regenerated.

## Round 4 — 2026-09-15

Before merging PR #221, the host made two purely cosmetic post-approval
fixes to the ADR text (cleaned up a shell-escaping artifact in the
Phase 4 `rg` command example; reflowed two paragraphs with a mid-sentence
line-wrap artifact — no wording, claim, fix, or file list changed). This
changed the file's SHA256, invalidating the round-3 approval record.
`runner.py check` confirmed the mismatch mechanically before this round
ran.

- Session id: 01a0a645-761d-7713-8e5d-bbf3baf3c283 (resumed)
- Plan SHA256: 227e9a404f7c13e235fdb3754cfa13c2764446d53d89442e11f18fbf9e3a7247
- Verdict: **APPROVED**
- Summary: the command cleanup and paragraph reflow preserve the
  approved migration requirements; R1 and R2 remain closed.

### Outcome
ADR-0003 is **APPROVED** by independent Codex review as of plan SHA256
`227e9a404f7c13e235fdb3754cfa13c2764446d53d89442e11f18fbf9e3a7247` (the
exact content merged to `main` in PR #221). Approval is bound to that
exact plan path and hash — any further edit to the ADR requires another
review round. No implementation is authorized by this approval; it
covers the plan only.

## Build — Phase 0a — 2026-09-15

User authorized implementation, scoped to Phase 0a only (metadata +
checker + generated map; zero directory moves) after being asked to
choose a scope given the ADR's own phased, multi-PR migration plan. See
`PHASE-0A-SPEC.md` for the derived, scoped build work order — it is a
narrower restatement of ADR-0003's own "Phase 0a in detail" section, not
independently re-reviewed as its own document, so the build runs under
`--unreviewed-spec` per the shared build reference (design review already
covers this text via the ADR approval above; this only scopes execution
to a subset of it and does not add new requirements).

Build ran via `runner.py build --host claude --builder codex`, base
commit `9177bc910` (the prep commit above), in an isolated worktree at
`../rusty_mill-adr0003-phase0a` (kept separate from the primary checkout,
which had an unrelated pre-existing untracked file). Codex session
`01a0a657-7eb7-78e3-8a85-20a8e9d1aac6`, exit 0, 482s, 246 files changed
(239 manifests + 4 new scripts + `docs/WORKSPACE-MAP.md` + `ci.yml` +
`README.md`). Codex's own report noted two deviations: `/tmp/metadata.json`
was unwritable in its sandbox (used a checkout-local temp path instead —
immaterial), and `cargo fmt --all -- --check` hit a Windows argv-length
limit (worked around with per-package batches).

## Inspect — Phase 0a — 2026-09-15

Independent inspection by the coordinating Claude host (not a separate
fresh CLI session — see rationale below). Did not trust Codex's own
proof report; re-ran everything and additionally verified the one thing
its report couldn't attest to (per-crate correctness against Appendix B).

- `git status --short` in the worktree: exactly 246 changed paths, 241
  modified (`ci.yml`, `README.md`, 239 manifests) + 5 untracked new files,
  **zero** renames/deletions — confirms no directory moves happened,
  matching the "out of scope" list in `PHASE-0A-SPEC.md`.
- `git diff --numstat` over all 239 crate `Cargo.toml` files: 717 lines
  added (= 239 × 3, one `[package.metadata.rusty_mill]` table + one
  `layer` line + one blank line, each), **zero** lines deleted anywhere
  — confirmed by hand on 3 spot-checked files across layers
  (`rusty_std` → foundation, `crates/rustils/crates/platform` →
  platform, `rusty_boot` → tools) and programmatically for the rest.
- Wrote and ran a standalone script cross-checking every one of the 239
  workspace members' actual `metadata.rusty_mill.layer` (from a fresh
  `cargo metadata` run against the built worktree) against ADR-0003
  Appendix B's `layer-map` block, parsed directly from the approved ADR
  text: **0** members in Appendix B but not a workspace member, **0**
  workspace members not in Appendix B, **0** layer value mismatches,
  **0** members with no layer set. This is the one correctness property
  neither `check_workspace_layers.py` nor `generate_workspace_map.py`
  can verify on their own (they enforce structural validity and
  dependency direction, not that a specific crate got the specific
  layer Appendix B calls for) — closed by this direct cross-check.
- Read `check_workspace_layers.py` and `generate_workspace_map.py` in
  full: both match `PHASE-0A-SPEC.md`'s Task 2/3 requirements (upward-edge
  check keyed off Appendix A's literal order; apps cross-family check
  correctly scoped to `apps`-layer callers only, with `tools` callers
  exempted by construction rather than a special case, matching R2's
  fix; dependents count is a deduplicated per-crate set, matching the
  "distinct other members" framing; map sorted by layer/family/name).
  Read both new test files in full: substantive, cover the cases
  `PHASE-0A-SPEC.md` asked for (missing/invalid layer, all three
  dependency kinds, same- vs. cross-family apps edges, the
  `rusty_boot`→`rush` exemption case by name, external non-member edges
  ignored, Windows-style manifest paths) plus extra cases the spec
  didn't require (Unicode descriptions, CRLF drift detection, sorted
  output independent of input order).
- Read the full `README.md` diff: only the "How the crates relate"
  section (2 insertions, 883 deletions) changed; the 239-row crate table
  and the "History" section are untouched, matching Task 4.
- Read the full `ci.yml` diff: 2 new steps appended to the existing
  `dependency-policy` job, reusing its existing `/tmp/metadata.json` —
  matches Task 5, no new job.
- Ran the proof commands myself (not just re-reading Codex's report):
  `python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v` →
  58 passed; `cargo metadata --format-version=1 --all-features --locked`
  → succeeds (pre-existing, unrelated `[profile]`-on-non-root-package
  warnings only); `check_workspace_deps.py` and `check_workspace_layers.py`
  against real metadata → both exit 0, no violations;
  `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md` → exit 0,
  committed file matches freshly generated bytes.
- Independently reproduced the `cargo fmt --all -- --check` Windows
  argv-length failure Codex reported, and confirmed it is **pre-existing
  and unrelated to this PR**: it fails identically on `main` at the exact
  commit this branch is based on (`97896e1ad`), and zero `.rs` files are
  touched by this diff, so formatting cannot have regressed. CI's `fmt`
  job runs on `ubuntu-latest` and won't hit this Windows-only limit.

No findings. Did not spin up a separate fresh-Claude `inspect` CLI
session — the coordinating host already achieved full coverage of the
changed surface through the checks above (a complete programmatic
cross-check of all 239 manifests plus a full read of every hand-written
line Codex added, not a sample), which is what a fresh session would
otherwise be reaching for; the shared build reference frames a separate
fresh-CLI pass as available when useful, not mandatory for the
Claude-host/Codex-builder path. Flagging this choice here rather than
silently treating it as equivalent to a fresh-session review.

### Outcome
Phase 0a build **verified correct** as of the worktree branch
`claude/adr-0003-phase0a-2026-09-15` (prep commit `9177bc910` +
Codex's uncommitted working-tree changes at inspection time). Not yet
committed by Codex (per the build reference, the builder does not
commit/push/publish) or by the host. Ready for the host to commit, open
a PR, and — per the known-cost note in `PHASE-0A-SPEC.md` — expect the
full CI build/test/clippy/cross-compile matrix to run, since every one
of the 239 manifests changed.

PR #222 opened, full CI matrix green (clippy × 2 OS, test × 6 shards,
cross-compile, npm build, all passed), merged by the user at `aa9e32b6a`.

## Build — Phase 0b — 2026-09-15

User set a session goal ("after merge continue until fulfilled") to
carry the ADR-0003 migration through its remaining phases without
stopping to ask at each one; this and subsequent phase entries proceed
under that standing authorization rather than a fresh per-phase ask,
unless something genuinely needs a human decision (see the "stop and
report" cases called out in each phase's spec).

Computed the Phase 0b cross-family entry set independently, twice: once
before Phase 0a merged (247 entries / 85 manifests / 53 targets, 13
already hoisted / 40 new), and again fresh against the post-merge base
commit `aa9e32b6a` in a new worktree (`../rusty_mill-adr0003-phase0b`,
branch `claude/adr-0003-phase0b-2026-09-15`) — identical numbers both
times. Cross-checked against the ADR's own historical estimate (128/85/
257): the 85-manifest count matches exactly both times; the entry count
is off by 10 (247 vs 257), most likely a minor counting-methodology
difference against a script that was never committed to the repo, not a
correctness issue — the manifest-level count (which manifests actually
need edits) is what matters for scoping the work, and it matches
exactly. Verified zero version-constraint disagreements and zero
root-relative-path inconsistencies across all entries for the same
target crate — a clean set with no ambiguous cases to resolve by hand.
Notable finding: 13 of the 53 distinct target crates already have a
`[workspace.dependencies]` entry at root (this workspace has already
been partially, organically hoisted over time) — only 40 need a new
entry.

Wrote `PHASE-0B-DATA.json` (the exact 247-entry set, machine-readable)
and `PHASE-0B-SPEC.md` (the work order, built to use that data file as
ground truth rather than asking Codex to rediscover it) and committed
both as the prep/baseline commit before delegating (`e1c8520cc`).

### Round 1 — blocked, not a defect in the spec's discipline

Codex's build (session `01a0a68c-21ab-75e1-b2d3-766acb8c2c3d`, exit 0,
303s) applied all 247 edits, then hit a real Cargo restriction running
its proof commands: `default-features = false` cannot be set on a
`workspace = true` dependency unless the workspace-level entry itself
also disables default features. `rush`'s dependency on `rusty_lines`
(one of the 247) does exactly that. Codex correctly stopped, reported
the blocker with full detail, proposed but did **not** apply a fix
(changing root `rusty_lines` to `default-features = false` and
compensating at `rusty_boot`), and left the working tree available for
review rather than forcing something through — exactly the discipline
`PHASE-0B-SPEC.md` asked for on any case needing a human call.

Investigated directly: found the *complete* set by scanning every one of
the 247 entries for `extra_keys["default-features"] is False` — exactly
6 (not just the one Codex's proof run happened to hit first: `rush`→
`rusty_lines`, `rusty_ansder`→`rusty_wire`, `rusty_oauth`→`rusty_json`,
`rusty_request`→`rusty_json`, `rusty_rag`→`rusty_simd`, `rusty_uuid`→
`rusty_serde`). Rejected Codex's proposed fix (lowering a shared root
default to satisfy one minority consumer changes behavior for every
*other* consumer of that crate — a real semantic decision, not a safe
mechanical hoist, and the ADR gave Phase 0b no mandate to make that
call). Chose instead to exclude exactly those 6 entries from the hoist,
leaving each as an unchanged direct `path` dependency — checked first
that none of the 5 affected target crates would lose *all* their
cross-family consumers by doing so (they don't: `rusty_serde` drops out
of scope entirely since `rusty_uuid` was its only cross-family consumer,
`rusty_lines` and `rusty_simd` still get a root entry because
`rusty_boot`/other consumers remain, `rusty_wire` and `rusty_json` were
already-hoisted targets unaffected either way).

Reset the worktree to the clean prep baseline (`e1c8520cc`, discarding
Codex's blocked partial edits — all of it was this session's own
just-attempted work, nothing to preserve) and regenerated
`PHASE-0B-DATA.json` v2 against that same baseline: **241 entries to
hoist / 83 manifests touched / 52 distinct targets (12 already hoisted,
40 needing a new root entry) / 6 entries excluded with a documented
reason**, `targets_already_hoisted`/`targets_needing_new_entry` counts
verified against the *pre-Codex-edit* root `Cargo.toml` specifically
(the first pass at this recomputation accidentally checked against
Codex's already-modified working tree and produced nonsense — caught and
fixed before writing the final file). Updated `PHASE-0B-SPEC.md` to
document the v2 data file, the exclusion rule, and the reason, and
resumed the same build session with this fix.

Attempting `--resume` on the round-1 build result failed fast:
`claudex-loop: Checkout changed since the previous build. Inspect
intervening work before continuing.` — correct behavior, since the
worktree had just been reset and re-committed to a new baseline
(`192da737a`) out from under that session's recorded fingerprint.
`--resume` is for iterating on the *same* baseline in response to
findings, not for a host-advanced baseline; switched to a fresh (not
resumed) `build` call against `192da737a` instead, relying on
`PHASE-0B-SPEC.md` v2 being fully self-contained rather than on
conversational continuity with the round-1 session.

### Round 2 — succeeded

Codex's build (session `01a0a694-c334-7462-9027-49d5f766df69`, exit 0,
310s) applied 241 hoists across 83 manifests, added 40 root entries,
left the 6 exclusions untouched, deleted `PHASE-0B-DATA.json` as
instructed, and reported its own dependency-graph-identity check passed.

## Inspect — Phase 0b — 2026-09-15

Independent inspection by the coordinating Claude host, same rationale
as Phase 0a for not spinning up a separate fresh-CLI session (full
programmatic coverage achieved directly below, not a sample).

- `git status --short`: exactly 85 changed paths (83 manifests + root
  `Cargo.toml` + `PHASE-0B-DATA.json` deletion), **zero** renames.
- `git diff --numstat` over all 83 touched manifests: added-lines equals
  deleted-lines on **every single one** — confirmed programmatically,
  not sampled — meaning every touched manifest is a pure
  `path = "..."` → `workspace = true` line swap, nothing else changed.
- All 6 excluded entries individually verified by reading the actual
  lines: `rush`'s `rusty_lines` (still `path = "../rusty_lines",
  version = "0.4.0", default-features = false`, byte-identical to
  before), `rusty_oauth`'s and `rusty_request`'s `rusty_json` (both
  still `path = "..."`, `default-features = false` intact), `rusty_uuid`'s
  `rusty_serde` (still `path = "../rusty_serde/rusty_serde"`), and
  `rusty_ansder`/`rusty_rag` — the two manifests whose *only*
  cross-family entry was excluded — confirmed to have **zero** diff at
  all (`git diff --stat` empty for both), meaning they were correctly
  left out of the 83 touched files entirely.
- Read the full root `Cargo.toml` diff: exactly 40 new lines in a single
  insertion block inside the existing `[workspace.dependencies]` table,
  alphabetically placed, no reordering or reformatting of any existing
  entry; `version` present only on the entries whose *remaining* (post-
  exclusion) consumers actually specified one (confirms the version-hoist
  logic re-evaluated against the post-exclusion consumer set, not the
  original 247 — `rusty_lines`'s only `version` mention came from the
  now-excluded `rush` entry, and correctly carries no `version` in its
  new root entry).
- **Dependency-graph identity, both modes** (the strongest correctness
  gate specified): diffed `cargo metadata --format-version=1
  --all-features --locked` before vs. after — 1701 nodes both times, 0
  added, 0 removed, **0** with a changed resolved-`deps` or
  resolved-`features` tuple. Repeated with plain `cargo metadata
  --format-version=1 --locked` (no `--all-features`, the more sensitive
  check for the excluded `default-features`/`optional` entries,
  generated by temporarily `git stash`-ing the Phase 0b diff to get a
  true "before" snapshot from the exact same worktree rather than a
  separately-cloned one) — 1394 nodes both times, again **0** changed.
  This is byte-for-byte confirmation the hoist has zero build-behavior
  effect in either feature-resolution mode.
- Ran the proof commands myself: `python3 -m unittest discover -s
  .github/scripts -p 'test_*.py' -v` → 58 passed (script untouched by
  this phase, as expected); `check_workspace_deps.py` and
  `check_workspace_layers.py` against fresh metadata → both exit 0 (the
  former needs `PYTHONUTF8=1` on this Windows host for the same
  pre-existing, CI-irrelevant locale reason noted in the Phase 0a
  inspection).

No findings.

### Outcome
Phase 0b build **verified correct** as of the worktree branch
`claude/adr-0003-phase0b-2026-09-15`. Ready to commit, open a PR, and —
per `PHASE-0B-SPEC.md`'s known-cost note — expect a partial (not full)
CI matrix scoped to the 83 touched manifests and their dependents.

PR #223 opened, full-required-checks green, merged by the user at
`0783ef859`.

## Build — Phase 1 — 2026-09-15

First phase involving actual `git mv` directory moves (18 crates: the
`rustils`/`rustils_async` platform-half split plus the intact
`rusty_test` → `portable-runtime` rename/move). Larger blast radius than
0a/0b, so did more upfront verification before writing the spec rather
than leaning on the build round to discover problems:

- Confirmed `coreutils`/`coreutils-async` (the `apps`-layer halves of the
  `rustils`/`rustils_async` split, deferred to Phase 4) already reference
  every platform crate they depend on via `workspace = true` — a direct
  payoff of Phase 0b's hoisting: moving the platform crates out from
  under them needs zero edits to their manifests, only a root
  `Cargo.toml` path update.
- Swept the entire workspace (not just cross-family, all 239 members)
  for any remaining literal `path = "..."` dependency targeting one of
  the 18 moving crates: found exactly 3, all same-family internal
  siblings within `rustils_async` (`platform-async-linux` → `platform-
  async`/`reactor-core`, `platform-async-mock` → `platform-async`) —
  safe, they move together and the relative paths stay valid.
- Read both stale nested `crates/rustils/Cargo.toml` and `crates/
  rustils_async/Cargo.toml` in full: confirmed both are genuinely dead
  pre-merge `[workspace]` roots (their own `[workspace.package]` version/
  repository fields describe the standalone-repo history), safe to
  delete per the ADR's own instruction.
- Checked every crate's own manifest for how it references same-family
  siblings: all already use `X.workspace = true`, none use a literal
  relative path (except the 3 above) — meaning moving the crates needs
  no per-crate manifest edits at all, only root `Cargo.toml`'s 18
  member-path entries + 12 `[workspace.dependencies]` path entries + 1
  `exclude` entry.
- Identified the 6 Phase-1 crates with zero root `[workspace.dependencies]`
  entry today (`platform-async-mock`, `reactor-core`, `threading`,
  `proc-runner`, `pty-shell`, `stat-tool`) and confirmed each has zero
  consumers anywhere except same-family siblings (fine) or the two
  stale manifests being deleted (moot) — none needs a new root entry.
- Grepped every `.rs` file in the repo for a hardcoded `rustils`/
  `rusty_test` path: found exactly one,
  `crates/rusty_test/crates/conformance/tests/layering.rs` (`GROUP_PREFIX`
  + a fixed-depth `ancestors().nth(4)` workspace-root lookup that is only
  correct before the move — the new path is one level deeper). Located
  the exact robust pattern already used by the Nexus guards
  (`dep_invariants.rs`'s own `workspace_root()`, a walk-up-until-
  `[workspace]`-found loop) to copy in as the fix.
- Grepped every `.md` file for a `rustils`/`rustils_async`/`rusty_test`
  path outside those three directories themselves: found exactly 4 —
  `CHANGELOG.md` and `PLAN-REVIEW-LOG.md` (dated log entries, historical,
  excluded), `docs/adr/0003-workspace-layout-by-layer.md` (the approved
  plan itself, excluded — its Appendix B "current directory" column is a
  deliberate before/after snapshot, not a live link), and `README.md`
  (the 239-row crate table — real Markdown links that do break on a
  move; confirmed exactly the 18 rows in question need both their link
  target and their backtick-quoted path updated). Confirmed
  `ARCHITECTURE.md`/`CONTRIBUTING.md` mention none of these paths, and
  none of the 11 READMEs the ADR's own review flagged as having a
  cross-family link (`mill-term`, `rusty_ansder`, etc.) point at any of
  these three families either — their broken links must be to families
  moving in a later phase, out of scope here.

Wrote `PHASE-1-SPEC.md` baking in every fact above as ground truth (not
asking Codex to re-derive any of it, only to sanity-check and report if
something looks off) and committed it as the prep/baseline commit before
delegating. Captured a pre-move `cargo metadata --all-features --locked`
snapshot at `C:\tmp\phase1-metadata.json` for the post-move
dependency-graph-identity check.

### Round 1 — blocked by environment, not the spec

Codex's build (session `01a0a6ca-7d69-7cb3-8d52-3a30d71ca443`, exit 0,
120s) never got to apply anything: `git mv` failed with a Windows
permission error creating `.git/worktrees/rusty_mill-adr0003-phase1/
index.lock`. That file lives *outside* the worktree's own directory for
a linked `git worktree` (it's under the main checkout's `.git/`), so it
sits outside Codex's sandbox root regardless of how permissive the
sandbox is *inside* the worktree — an environment/tooling limitation,
not something the spec could have avoided. Cleaned up the empty
directories Codex's blocked attempt left behind before proceeding.

Disposition: performed every move in `PHASE-1-SPEC.md`'s tables myself
directly (`git mv` for 18 crate directories + each family's shared
docs/CI files, `git rm` for the 2 stale nested workspace manifests) —
mechanical, zero judgment calls, matches the spec's tables exactly.
Verified before committing: `git status --short` showed 234 renames +
2 deletions, nothing else; every rename in `git status -M` at 100%
similarity. Committed as its own commit (`10c074f59`) so the move is a
clean, separately-reviewable unit from the content edits still to come.
Verified `git log --follow` traces history through the rename for two
spot-checked files (`platform/Cargo.toml`, `conformance/tests/
layering.rs`) — reaches back to pre-merge commits (`9973c73d4`,
`af3a383b9`), confirming history survived.

Updated `PHASE-1-SPEC.md` with a "Status" section marking the move done
and scoping the next build round to only the remaining content edits
(root `Cargo.toml`, `layering.rs`, `README.md`), so a resumed/fresh
build wouldn't re-attempt `git mv` against the same permission wall.
Committed (`279425569`) and launched a fresh build against that base.

### Round 2 — succeeded (with one legitimately-flagged spec gap)

Codex's build (session `01a0a6ce-218b-7d33-8df0-a530e2ab7062`, exit 0,
216s) applied all three content edits and ran what proof it could
(`python3` was unexpectedly unavailable in its sandbox this round — ran
it myself instead, see Inspect below). It flagged, correctly, that my
own acceptance criterion's blanket grep sweep (for `"crates/rustils/"`
etc. across every tracked file) would fail even on a fully-correct
build, because it also matches: (a) the deliberately-still-old
`coreutils`/`coreutils-async` paths in root `Cargo.toml` and
`README.md` (correct — those two crates don't move until Phase 4), and
(b) `docs/adr/0001-consolidate-crates-into-workspace.md`, a historical
document citing old paths as history, which the spec's exemption list
missed (it named `docs/adr/0003...`, `CHANGELOG.md` and
`PLAN-REVIEW-LOG.md` but not ADR-0001 or `RELEASE_NOTES.md`, both of
which the *original* ADR-0003 text already exempts). Codex proposed
adding grep exceptions rather than editing any of these files — correct
call, deferred to the host rather than applied unilaterally.

## Inspect — Phase 1 — 2026-09-15

Independent inspection by the coordinating Claude host, same rationale
as prior phases for not spinning up a separate fresh-CLI session.

- Re-ran the same blanket grep myself, properly excluding
  `docs/adr/0003-workspace-layout-by-layer.md`, `CHANGELOG.md`,
  `PLAN-REVIEW-LOG.md`, and (newly identified) `RELEASE_NOTES.md`,
  `CODEX-MONOREPO-REVIEW*.md`, `docs/adr/0001*`, and `PHASE-1-SPEC.md`
  itself. Categorized every remaining hit by hand, one by one: all of
  them are either a historical document properly describing past state,
  my own spec file's before→after tables, or a legitimate
  `coreutils`/`coreutils-async` reference that is supposed to still
  point at the old path this phase. **Zero unexplained hits** — the
  build has no missed reference; the spec's acceptance criterion was
  just too blunt.
- `git status --short`: exactly the 3 intended files modified
  (`Cargo.toml`, `README.md`, `layering.rs`), nothing else, confirming
  Codex didn't touch anything beyond its scoped edits on top of the
  host's move commit.
- Read the full `Cargo.toml` diff: 18 member-path entries, 12
  `[workspace.dependencies]` path entries, and the 1 `exclude` entry all
  updated to the correct new paths, `coreutils`/`coreutils-async`
  correctly untouched; `git diff --numstat` shows 31/31 (pure swap, no
  stray edits).
- Read the full `README.md` diff: exactly 18 rows changed (`git diff
  --numstat` 18/18), both the link target and the backtick path on each;
  `coreutils`/`coreutils-async` rows correctly untouched.
- Read the full `layering.rs` diff: `GROUP_PREFIX` updated, and
  `workspace_root()` replaced with the exact walk-up-until-`[workspace]`
  pattern copied from the Nexus guards, matching the spec's instruction
  precisely.
- Ran the proof commands myself (Codex's `python3` was unavailable this
  round): `python3 -m unittest discover -s .github/scripts -p 'test_*.py'
  -v` → 58 passed; `check_workspace_deps.py` and
  `check_workspace_layers.py` against fresh metadata → both exit 0 (the
  latter re-validates every layer/family assignment against the *actual*
  post-move manifest directories — a strong signal the move landed each
  crate where the ADR says it belongs).
- **Dependency-graph identity**, compared by package name rather than
  resolve-node id (node ids embed the manifest path, which the move
  necessarily changes, so a name-keyed comparison is the correct
  invariant here): 1443 distinct package names before and after, **0**
  with a changed dependency or feature set.
- Ran the one test whose own logic changed, not just its build:
  `cargo test -p conformance --test layering` — all 4 tests pass from
  the new location, including `every_workspace_member_is_assigned_a_layer`,
  which specifically exercises the new dynamic `workspace_root()`.
- Compiled a representative sample directly (`cargo check -p platform -p
  platform-linux -p winargv -p coreutils -p coreutils-async`) — all five
  succeed, including the two crates that stayed behind at their old path
  while consuming the now-moved platform crates via `workspace = true`,
  the strongest end-to-end confirmation the split works as designed.

No findings beyond the spec's own grep-criterion gap, which is closed
by inspection rather than a code change (nothing needed fixing; the
criterion needed narrowing).

### Outcome
Phase 1 build **verified correct** — 18 crates + family-level files
moved (mixed authorship: host performed `git mv`/`git rm`, Codex
performed the content edits; both inspected by the host, matching the
build reference's mixed-authorship logging requirement). Ready to
commit the remaining edits, open a PR, and expect a large but partial
CI matrix (the 18 moved crates plus their dependents).

## CI failure — PR #224 — 2026-09-15

Full CI ran (root `Cargo.toml`'s member/`workspace.dependencies` *path*
edits are not a pure addition, so `cargo_toml_diff.py` correctly
classified this as unsafe and forced the full sweep, not just the moved
crates' dependents — expected, matches the CI header comment's own
documented policy). Two independent failures surfaced, investigated
separately:

**1. `dependency policy` job failed — a real bug this phase exposed, not
introduced.** `generate_workspace_map.py --verify docs/WORKSPACE-MAP.md`
reported staleness. Root cause: `check_workspace_layers.py`'s
`package_family()` (written in Phase 0a, before any crate had moved)
takes the *second* path component after `crates/` as the family. Before
this phase, every member lived at `crates/<family>/...`, so that was
correct. Now that `platform-linux` etc. live at
`crates/platform/rustils/crates/platform-linux`, the second component is
`platform` — the *layer*, not the family — so the function silently
started returning the layer name instead. This doesn't affect
`check_workspace_layers.py`'s own violation detection this phase (the
cross-family apps check only fires between `apps`-layer crates, and none
of the 18 moved crates are `apps`-layer), so the 0-violations result I
verified earlier was still correct — but it does mean the same bug would
have **silently disabled the cross-family apps check entirely** once
Phase 4 moves `apps`-layer crates under `crates/apps/...` too (every
`apps` crate would collapse into one fake family named `apps`,
permanently zero cross-family findings possible). Caught now instead,
before it could hide a real Phase 4 violation. Fixed `package_family()`
to skip the layer-name component when present (`parts[1] in LAYER_ORDER
=> family = parts[2]`), added 3 unit tests covering the moved,
single-crate-under-layer, and not-yet-moved shapes (61 tests total, up
from 58), regenerated `docs/WORKSPACE-MAP.md` with the corrected family
values, and reran `--verify` (exit 0).

**2. 3 `test (windows-latest, ...)` shards failed — pre-existing
flakiness, unrelated to this PR.** Two failures, both in
`sessionmgr-daemon::worktree_lifecycle` (`closing_with_discard_throws_
the_worktree_and_branch_away`, `two_worktree_sessions_on_one_repo_are_
independent`), each already retried twice by nextest before failing a
third time (`TRY 3 FAIL`); two *other*, different tests in the same run
(`rusty_tls`'s async TLS tests) were marked `FLAKY 2/3` (passed on
retry) in the same job. Checked `sessionmgr-daemon`'s dependency graph
directly: zero direct or transitive dependency on any of the 18 moved
crates (`rusty_tokio`, `serde`, `serde_json`, `sessionmgr-*` siblings,
`ureq`, `thiserror`, `embed-manifest` — nothing in `rustils`/
`rustils_async`/`portable-runtime`). This is Windows-CI git-worktree
timing flakiness in an unrelated crate family, surfaced only because the
root-`Cargo.toml` path edit forced the full sweep. Re-running the failed
jobs rather than treating this as a Phase 1 regression.

Pushed the `package_family()` fix (`fb259c193`); the re-run's
`dependency policy` job passed, confirming the fix. The re-run's own
Windows shards then failed differently: `test (windows-latest, shard 2)`
on `nexus-types::sandbox::tests::workspace_write_roots_include_cwd_
extras_and_tmp` (asserts a Unix-style `/tmp` path is present in a
Windows sandbox-roots list — an inherently Windows-hostile assertion)
after an `install-action: installation failed due to bash startup
failure` warning earlier in the same job (a documented GitHub Actions
partner-runner-image bug, `actions/partner-runner-images#169`); `test
(windows-latest, shard 3)` on `nexus-terminal::job_object::imp::tests::
assign_to_current_process_succeeds_once` (`TRY 3 FAIL` — Windows job-
object assignment, sensitive to whether the runner process is already in
a job object, a known flaky class). Confirmed both `nexus-types` and
`nexus-terminal` have zero dependency relationship to any of the 18
moved crates. User merged PR #224 with these two checks red — a
reasonable call given both are pre-existing, unrelated Windows-CI
flakiness in the `nexus` family, and the migration's own behavior was
already independently verified (dependency-graph identity, `cargo
check`/`cargo test` on the actual changed surface) before either failure
surfaced.

## Build — Phase 2 — 2026-09-15

Continuing under the same standing authorization (see Phase 1's build
entry). Computed the foundation-layer move set from Appendix B: 30
crates across 26 families (matches the ADR's own count exactly).
Performed the `git mv` directly again without attempting a Codex round
first — the sandbox limitation is now confirmed twice, no reason to
spend a build round rediscovering it. 26 family directories moved
wholesale (`crates/<name>` → `crates/foundation/<name>`, each carrying
its own nested sub-crates automatically); deleted the third stale nested
workspace manifest, `crates/rusty_serde/Cargo.toml`. Verified: 350
renames at 100% similarity, 1 deletion, nothing else. Committed as
`30afc0c9e`.

Swept the whole workspace for remaining literal `path` dependencies
targeting any of the 30 moved crates (same technique as Phase 1): found
9. Categorized each: 6 resolve for free because both the consumer and
the target are moving to the same new layer this phase, so their
relative path survives unchanged (`rusty_ansder`→`rusty_wire`,
`rusty_uuid`→`rusty_serde`, and 4 same-family
derive/erased-sibling references) — 3 are real, because the *consumer*
is a `libs`-layer crate not moving until Phase 3 while the *target* just
moved: `rusty_oauth`→`rusty_json`, `rusty_request`→`rusty_json`,
`rusty_rag`→`rusty_simd`. All 3 are among Phase 0b's 6
default-features-excluded entries (documented there), so this was
expected, not a surprise — Phase 0b's log already flagged that leaving
an entry unhoisted means its relative path needs a manual check whenever
either endpoint moves. Computed the exact fix for each
(`"../rusty_json"` → `"../foundation/rusty_json"`, etc.).

Also swept every `.rs` file for a hardcoded path to any of the 26
families (zero, unlike Phase 1's `layering.rs`) and every `.md` file for
a cross-family link into them (only `README.md`'s 30 crate-table rows;
`rusty_ansder`'s own README, despite being one of the ADR review's
11-file cross-family-link list, turned out not to reference any of these
paths directly).

Wrote `PHASE-2-SPEC.md`, explicitly requiring the `generate_workspace_map.py
--verify` check this time (the gap that caused Phase 1's first CI
failure) as an acceptance criterion up front rather than finding it again
via a red build. Committed and delegated the remaining edits (root
`Cargo.toml`, 3 manifests, `README.md`) to Codex.

Codex's build (session `01a0a7a9-40a9-7270-b6f6-da0933bca0c6`, exit 0,
260s) applied all 5 authorized edits correctly and additionally ran its
own grep sweep against the acceptance criterion — finding 6 stale
references the spec's exemption list didn't account for, and correctly
declining to touch any of them without authorization: `PHASE-1-SPEC.md`
(a merged historical spec the exemption list forgot to add), plus 5
*comment*-only references (not functional code/config) in `Cargo.toml`,
`.github/workflows/ci.yml` (×2), and `crates/rush/src/glob.rs` (×2) —
each a doc comment pointing a human reader at
`crates/rusty_libc/bench`, `crates/rusty_win32/Cargo.toml`,
`crates/rusty_config/Cargo.toml`, or `crates/rusty_regx/docs/
GLOB_DESIGN.md` for further reading, none of them affecting build
behavior. Fixed all 5 directly (one-line path-string edits, zero
functional risk) and added `PHASE-1-SPEC.md` to the exemption list;
re-ran the full grep sweep — zero unexplained hits.

## Inspect — Phase 2 — 2026-09-15

- `git status --short`: exactly 7 files (`Cargo.toml`, `README.md`, the
  3 manifests, plus the 2 comment-fix files) modified, nothing else.
- `git diff --numstat`: `Cargo.toml` 56/56, `README.md` 30/30 (pure
  swaps — 30 member paths + 24 `workspace.dependencies` paths + 1
  `exclude` entry + 1 comment = 56; 30 crate-table rows × 2 occurrences
  = 30 numstat-line-pairs).
- Read the 3 manual manifest fixes (`rusty_oauth`, `rusty_request`,
  `rusty_rag`) in full: each changes only the `path` value on its one
  `rusty_json`/`rusty_simd` line, every other key (`default-features`,
  `features`) untouched, matching the spec exactly.
- Ran the proof commands myself: `python3 -m unittest discover` → 61
  passed; `check_workspace_deps.py`, `check_workspace_layers.py`,
  `generate_workspace_map.py --verify` → **all three exit 0** this time
  (the map check specifically closes Phase 1's CI-failure gap).
- **Dependency-graph identity** by package name: 1443 names before and
  after, **0** changed — confirms the 3 manual path fixes preserved
  identical resolution, not just that they compile.
- Compiled the 3 manually-fixed crates plus a representative foundation
  sample directly (`cargo check -p rusty_oauth -p rusty_request -p
  rusty_rag -p rusty_std -p rusty_serde -p rusty_json`) — all succeed
  from the new locations.
- Investigated several IDE/background-analyzer diagnostics that
  surfaced against the *main checkout* (`rusty_tokio`/`rusty_stream`
  io-uring APIs, `rusty_fedora_agent`'s `platform_linux::LinuxSpawner`)
  during this work — confirmed unrelated: all are `cfg(target_os =
  "linux")`-gated APIs a Windows-hosted analyzer can't resolve,
  pre-existing and unaffected by any phase of this migration (`platform-
  linux` still resolves correctly via `workspace = true`, independently
  confirmed by the graph-identity check above).

No findings beyond the exemption-list gap, closed by the 5 comment
fixes above.

### Outcome
Phase 2 build **verified correct**. Ready to commit, open a PR, and
expect a large but partial CI matrix (the 30 moved crates plus their
dependents, plus `rusty_oauth`/`rusty_request`/`rusty_rag` and their own
dependents).

PR #225 merged by the user (`805796115`) with all checks that had a
conclusion green; the 3 Windows test shards were still running (not
cancelled by the merge) and their outcome wasn't chased down given the
established pattern from Phase 1's PR — same call, not re-litigated.

## Build — Phase 3 — 2026-09-15

Computed the libs-layer move set from Appendix B: 66 crates across 34
families (matches the ADR's own count exactly), 31 grouped into one of
7 themes (`ai`/`async`/`homelab`/`net`/`protocol`/`storage`/`ui`), 3
with no theme (`rusty_adk`, `rusty_git`, `rusty_wiremock`).

**Proactive fix before the move**: recognized that `libs/`'s thematic
subdirectories are a *second* wrapper level `package_family()` didn't
account for — the Phase 1 fix only skipped the layer segment, so
`rusty_tokio` at `crates/libs/async/rusty_tokio` would have derived
family `async` (the theme), repeating Phase 1's exact bug one level
deeper and re-silencing the cross-family apps check. Generalized
`package_family()` to skip a theme segment too when the layer is
`libs` and the next segment matches a known theme name; added 4 tests
(missing-theme, theme-present, theme-absent for `rusty_adk`,
theme-name-collision with another layer). Fixed and committed
(`a08d010fa`) *before* moving anything, closing this class of bug
proactively rather than discovering it via CI again.

Performed the `git mv` directly (34 family directories, creating the 7
theme subdirectories first). Verified: 1666 renames at 100% similarity,
no deletions. Noticed 8 markdown files (4 in `rusty_rusqlite`, 4 in
`rusty_gui`) got CRLF-normalized to LF as an incidental side effect of
being staged — matches this repo's own `.gitattributes` LF policy,
content unchanged, only `git log --follow` loses the thread at this
commit for those 8 files specifically (confirmed: all other 1658 files,
including every `.rs` and `Cargo.toml`, traced cleanly). Accepted as a
minor, disclosed side effect rather than something to engineer around.
Committed (`726e8398f`).

Swept the whole workspace for remaining literal `path` dependencies
targeting any of the 66 moved crates (reading manifests from the
pre-move commit, since the live filesystem no longer has the old
paths): found 17. 16 resolve for free (same-family, or both endpoints
moving to the same new theme this phase). 1 is real: `rush`→
`rusty_lines` — `rush` is an `apps`-layer crate not moving until
Phase 4, and this was the 6th and last of Phase 0b's `default-features`-
excluded entries (the other 5 were closed out in Phase 2), so finding
exactly one here was expected, not a surprise.

Also found 3 comment-only stale references via the standard sweep
technique (root `Cargo.toml`'s exclude-list comment, a doc comment in
`pty-shell/src/main.rs`, a comment in `rusty_key/crates/feed/Cargo.toml`)
— same category as Phase 2's 5 comment fixes, folded into the spec
directly this time instead of waiting for Codex to flag them. Also
checked and exempted: `docs/adr/0002-dependency-sovereignty-policy.md`
(a root ADR the original ADR-0003 text already exempts, which the
Phase 1/2 grep exemption lists hadn't included until now),
`crates/rusty_hister/docs/decisions/ADR-0002-*.md` (a per-crate
historical decision record), and `crates/rusty_multimodal_db/docs/
design/*.md` (prose mentions in backtick-code, not Markdown links — the
ADR's own rule that prose needs no edit, only broken links do).

Wrote `PHASE-3-SPEC.md` with all of the above baked in as ground truth.
Committed and delegated the remaining edits (root `Cargo.toml`,
`README.md`, `rush/Cargo.toml`, and the 3 comment fixes) to Codex.

### Round 1 — blocked by two real spec gaps, both found and correctly not silently fixed

Codex's build (session `01a0a7d0-eade-7d41-9225-5ea85923d356`, exit 0,
218s) applied every edit the spec actually specified, then found two
real problems the spec's Task 1/2 scoping missed, and proposed but did
**not** apply fixes for either — deferred both to the host:

1. **Task 1 was incomplete.** The host's workspace.dependencies sweep
   only checked whether each of the 34 *family* names was a root key —
   it never checked the actual *package* names of multi-crate families'
   sub-crates (`adk-core`, `rusty-db-core`, `rusty-search-core`, etc.,
   and `rusty-whisper`/`rusty-mcp` whose package name differs from their
   directory name). Re-derived the check properly (compare every one of
   the 66 members' actual `[package] name` against root
   `workspace.dependencies` keys, not just the 34 directory names): 48
   entries needed a path update, not 20 — the spec missed 28.
2. **Task 2 was incomplete in a different way**: the Phase 2 fixes for
   `rusty_oauth`/`rusty_request` (→ `rusty_json`) and `rusty_rag` (→
   `rusty_simd`) used `"../foundation/rusty_json"`-style paths correct
   for their *Phase-2* location (`crates/rusty_oauth/`). All three are
   themselves `libs`-layer crates that just moved *this* phase
   (`crates/libs/net/rusty_oauth/`, etc.) — one level deeper — so the
   same literal string now resolves to a nonexistent
   `crates/libs/net/foundation/rusty_json`. The host's Task-2 sweep
   only checked for literal paths *targeting* a Phase-3 crate; it never
   re-checked whether a Phase-3-moving crate's own *existing* literal
   paths (to an already-moved, earlier-phase crate) were still valid
   after the crate carrying them moved again. Confirmed Codex's exact
   proposed fix (`"../../../foundation/rusty_json"` /
   `"../../../foundation/rusty_simd"`) is correct by hand-tracing the
   new directory depth.

Also confirmed Codex's third flag (`crates/libs/async/rusty_tokio/docs/
decision-request-real-tokio-interop-bridge.md` citing
`crates/rusty_request/src/client.rs:609:30`) is a pasted historical
panic backtrace in a decision-record document — exempt, same principle
as the per-crate ADR exemption; rewriting a stack trace to match a path
that didn't exist when the panic happened would be actively wrong, not
a fix.

Applied all three corrections directly (28 additional root path fixes
via a small script matching each key precisely, rather than 28 manual
edits; 3 manual one-line path fixes in `rusty_oauth`/`rusty_request`/
`rusty_rag`). Re-ran the full grep sweep with the decision-doc exemption
added — zero unexplained hits.

## Inspect — Phase 3 — 2026-09-15

- `git status --short`: exactly 8 files modified (the 5 Codex touched
  plus the 3 cascading fixes), nothing else.
- `cargo metadata --format-version=1 --all-features --locked` succeeds
  (previously failed outright — `crates/libs/net/foundation/rusty_json/
  Cargo.toml` did not exist — confirming the bug was real, not
  theoretical).
- `check_workspace_deps.py`, `check_workspace_layers.py`,
  `generate_workspace_map.py --verify` — all exit 0.
  `docs/WORKSPACE-MAP.md` needed **no regeneration** this phase — its
  columns are layer/family/crate/description/dependents-count, none of
  which a pure directory move changes (unlike Phase 1, which changed
  `package_family()`'s *output* for the first time; Phases 2-3 run
  against the already-fixed function from the start, so the map never
  went stale).
- **Dependency-graph identity** by package name: 1443 names before and
  after, **0** changed — confirms both the 28-entry fix and the 3
  cascading fixes are correct, not just that `cargo metadata` no longer
  errors.
- Compiled `rusty_oauth`, `rusty_request`, `rusty_rag`, `rush` (the 4
  crates in the cascading-fix chain) plus a representative sample from
  the 28-entry fix (`adk-core`, `rusty-db-core`, `rusty-search-core`,
  `rusty_tokio`) directly — all succeed.
- `python3 -m unittest discover` → 64 passed (unchanged from prep,
  Phase 3 touches no script).

No findings beyond the two spec gaps, both closed above.

### Outcome
Phase 3 build **verified correct**. This phase closes the last of Phase
0b's 6 `default-features`-excluded entries (`rush`→`rusty_lines`) and
fully resolves the cross-phase cascading class of bug it exposed
(Phase-N's fix for a still-moving-later crate needs re-checking every
time that crate itself moves in a later phase). Ready to commit, open a
PR, and expect the largest partial CI matrix yet (66 moved crates,
including `rusty_tokio`/`rusty_http`/`rusty_search`, which much of the
rest of the workspace depends on).

## CI investigation — PR #226 — 2026-09-15

All 3 `test (windows-latest, ...)` shards failed; every other check
passed (both `ubuntu-latest` clippy/test, `fmt`, `dependency-policy`,
`plan`, cross-compile, npm build). Pulled the actual failure summaries:

- Shard 2: `nexus-types::sandbox::tests::
  workspace_write_roots_include_cwd_extras_and_tmp` — the exact same
  test and panic line (`sandbox.rs:341:9`) that failed on PR #224.
- Shard 3: `nexus-terminal::job_object::imp::tests::
  assign_to_current_process_succeeds_once` — same test that failed on
  PR #224.
- Shard 1: `sessionmgr-daemon::supervisor_restart_recovery::
  a_session_survives_the_daemon_being_killed_and_is_adopted_by_its_
  replacement`, `TRY 3 FAIL` — a new test, but the same crate and same
  flaky *class* (daemon/process-supervision lifecycle, inherently
  OS-timing-sensitive) as PR #224's `sessionmgr-daemon::
  worktree_lifecycle` failures.

Checked dependency relationships directly: `nexus-types` and
`nexus-terminal` have zero dependency on any Phase 3 family (same as
before). `sessionmgr-daemon` *does* depend on `rusty_tokio`, which moved
this phase — worth stating precisely rather than waving it off. But the
dependency-graph-identity check already proved `rusty_tokio`'s resolved
`deps` and `features` are byte-identical before and after this phase's
move (see Inspect above) — a pure path relocation cannot change the
runtime behavior of a crate whose resolved dependency graph is
unchanged. Combined with the test itself being about killing and
restarting an OS process and checking recovery (inherently timing/
scheduler-sensitive, not content-sensitive) and matching the exact same
flaky pattern already established on PR #224 in the same crate family,
this reads as the same pre-existing Windows-CI flakiness, not a Phase 3
regression — logged for the record rather than treated as fully
dismissed without cause.

PR #226 merged by the user (`0cead027b`) before the fresh CI run
(re-triggered by the log-only push above) had finished all shards.

## Build — Phase 4 — 2026-09-15 (final phase)

Computed the apps/tools move set from Appendix B: 125 crates across 21
families (matches the ADR's own count exactly) — 20 into `apps/`, 1
(`rusty_boot`) into `tools/`.

Reused every lesson from Phases 1-3 proactively rather than rediscovering
them:
- **Package names, not directory names**, for the
  `workspace.dependencies` sweep (Phase 3's 28-entry miss) — checked
  all 125 members' actual `[package] name`, not just the 21 family
  directory names: 46 need a path update, 79 have zero external
  consumers (verified via `workspace = true` grep, none missing a
  needed root entry).
- **Every mover's own literal paths, not just incoming edges**, for the
  cascading-fix sweep (Phase 3's cross-phase cascade) — found exactly
  one: `rush`→`rusty_lines`, the same entry fixed once already in
  Phase 3 (`"../rusty_lines"` → `"../libs/ui/rusty_lines"`), needing a
  second re-derivation now that `rush` itself moved a level deeper.
- **CI/config files, not just source and docs**, for the hardcoded-path
  sweep — found 3 *functional* lines in `.github/workflows/ci.yml`
  (`rusty_meshed`'s `data-mesh-monitor` job: change-filter,
  `working-directory`, `cache-dependency-path`) that would have
  silently broken that job for every future `rusty_meshed` PR.
- **The 7 Nexus test guards** flagged back during the original ADR
  review (Phase 1's spec explicitly deferred these, "nexus doesn't move
  until Phase 4") — confirmed all 7 already use the robust
  walk-up-to-`[workspace]` pattern (none use a fixed-`ancestors()`
  depth the way `layering.rs` did), so only their hardcoded
  `"crates/nexus/crates"`/`"crates/nexus/shell"` prefix *strings* need
  updating, not their logic. Also found and explicitly scoped *out* a
  separate, pre-existing, unrelated bug in
  `ipc_topic_prefix_invariant.rs` (a `"crates/nexus-bootstrap/src"`
  reference missing its `crates/nexus/crates/` prefix entirely — never
  valid, even before this migration) — not this migration's to fix.

Performed the `git mv` directly: 20 family directories to `crates/apps/`,
`rusty_boot` to `crates/tools/`, `rusty_inventrory` renamed to
`rusty_inventory` during its move (per the ADR's naming convention).
`coreutils`/`coreutils-async` moved from their nested location under
`crates/rustils/crates/`/`crates/rustils_async/crates/` (deferred there
since Phase 1) — removing the two now-empty leftover directories
completes the `rustils`/`rustils_async` split Phase 1 started; neither
directory exists anymore. Verified: 4786 renames at 100% similarity,
zero deletions (cleaner than Phase 3's move — no CRLF-normalization
side effects this time). Committed (`c8423476e`).

Wrote `PHASE-4-DATA.json` (125-entry member mapping, 46-entry
workspace.dependencies fixes, 4 exclude fixes, 7 Nexus guard fixes,
the `rush` cascading fix, 3 CI functional fixes, 3 comment fixes) as
machine-readable ground truth and `PHASE-4-SPEC.md` referencing it,
rather than transcribing 125+ entries into prose by hand (the exact
transcription-error risk that caused Phase 3's original 20-vs-48 gap).
Committed and delegated to Codex.

### Round 1 — succeeded on every authorized edit, correctly declined to guess on the rest

Codex's build (session `01a0a852-ee73-7ea3-982d-700aab889dd2`, exit 0,
306s) applied all 12 authorized files correctly (verified: dependency
graph identity held, all 23 Nexus guard tests passed even before the
host's own re-verification). It also ran its own grep sweep and found
real gaps, and — rather than guessing at any of them — reported each
with a proposed resolution and left the working tree exactly at the
12-file authorized boundary:

1. **`docs/WORKSPACE-MAP.md` is genuinely stale, not a false positive.**
   Two things changed real Family values this phase, not just paths:
   `coreutils`/`coreutils-async` finally landing in their own
   single-crate-family location (`apps/coreutils`) instead of the
   leftover `rustils`/`rustils_async` shell family name they'd been
   stuck showing since Phase 1 deferred their move; and the
   `rusty_inventrory` → `rusty_inventory` rename literally changing the
   family name for its 3 crates. The spec's own acceptance criteria
   said to regenerate the map "if stale," but the file wasn't in the
   explicit allowed-file list Codex was told to stay within — a
   self-contradiction in the spec, correctly flagged rather than
   resolved by guessing which instruction wins.
2. **`PHASE-4-DATA.json` deletion vs. the blanket "no deletion"
   instruction** — the spec's Status section said "do not attempt any
   file deletion" (meaning: don't re-do the crate moves), but Acceptance
   criteria separately said to delete this one build-artifact file
   before finishing. Same class of self-contradiction as #1.
3. **42 residual path references across 15 files**, reported with full
   file:line detail rather than edited (correctly, since none were in
   the authorized list) or silently ignored.

### Host disposition on the 42 residual references

Categorized all 42 by hand rather than blanket-fixing or
blanket-exempting:
- **6 in `README.md`** — confirmed all fall inside the "## History"
  section (git-subtree merge-wave narrative, dated prose) — same
  exemption class as `CHANGELOG.md`/`RELEASE_NOTES.md`. No edit.
- **8 in `crates/platform/rustils/docs/coreutils-gap-backlog.md`** —
  a dated record ("Recorded 2026-07-21") citing specific file:line
  evidence for bugs found at that time — rewriting the paths would
  misrepresent what was true when recorded, the same reasoning that
  exempted Phase 1's decision-request panic backtrace. No edit.
- **3 in `.github/scripts/test_check_workspace_layers.py` /
  `test_cargo_toml_diff.py`** — synthetic test fixture data
  (`test_check_workspace_layers.py`'s own `rusty_boot` case
  deliberately exercises the *not-yet-moved* shape; `test_cargo_toml_
  diff.py`'s fixture is unrelated example data for a different check).
  No edit — these are intentionally testing a shape other than "moved."
- **25 across 9 files** — `ARCHITECTURE.md` (a live, prescriptive
  architecture description, not history), 4 more doc-comment mentions
  in the already-being-edited Nexus guard files (their functional
  string constants were already fixed; these were leftover
  *explanatory* comments citing the same stale path), `rusty_hister`'s
  own `AGENTS.md`/`README.md`, `rusty_meshed`'s own
  `capability-manifest.md`/`data-mesh-monitor/{README.md,src/
  reverseTrace.js}` (a crate's own live docs describing its own current
  structure — not historical, meant to be accurate now), a
  cross-crate comment in `rusty_kafka/Cargo.toml` citing
  `rusty_meshed`'s manifest path, and a comment in `portable-runtime`'s
  `proc-runner/src/main.rs`. Applied all 25 via a verified,
  count-checked script (each old string's occurrence count logged and
  matched the residual-sweep report's own per-file counts exactly: 2,
  2, 1, 1, 2, 8, 6, 1, 1, 1, 1).
- Regenerated `docs/WORKSPACE-MAP.md` (exactly the 5 rows Codex's own
  candidate predicted) and deleted `PHASE-4-DATA.json`.

## Inspect — Phase 4 — 2026-09-15

- `git status --short`: 22 files (12 originally authorized + the map
  regeneration + the data-file deletion + 9 additional comment/doc
  fixes), all accounted for.
- Re-ran the full grep sweep with every exemption applied (including
  the newly-identified historical `coreutils-gap-backlog.md` and the
  test-fixture files): **zero** unexplained hits (one apparent hit,
  `PHASE-0A-SPEC.md`, turned out to be a gap in the verification
  command's own exemption regex — `PHASE-[0-9]-SPEC.md` doesn't match
  `PHASE-0A-SPEC.md`'s letter suffix — not a real finding).
- `python3 -m unittest discover` → 64 passed; `check_workspace_deps.py`,
  `check_workspace_layers.py` → both exit 0;
  `generate_workspace_map.py --verify` → exit 0 after regeneration.
- **Dependency-graph identity** by package name: 1443 names before and
  after, **0** changed — confirms the 12 authorized edits, the 25
  additional comment fixes, and the map regeneration are all exactly
  what they claim to be (no accidental behavior change hiding in any of
  them).
- Ran all 7 Nexus guard test files myself (not just re-reading Codex's
  report): 23/23 passed, matching exactly — `dep_invariants` (3),
  `plugin_contract_purity` (5), `tauri_command_boundary` (5),
  `bootstrap_coverage` (1), `core_plugin_loc_budget` (3),
  `ipc_topic_prefix_invariant` (5), `dep_invariants_shell` (1). These
  aren't just "does it compile" checks — they assert real architectural
  invariants (microkernel isolation, contract/impl separation, IPC
  command registration) that now hold at the new location.

No findings beyond the two spec self-contradictions, both closed by
host disposition above (not code changes — the spec needed correcting
authority, not the build).

### Outcome
Phase 4 build **verified correct** — the final phase of ADR-0003's
migration. Every one of the 239 workspace members is now under its
assigned layer directory (`foundation/`, `platform/`, `libs/`,
`apps/`, `tools/`), matching Appendix B exactly. `crates/rustils/` and
`crates/rustils_async/` no longer exist — the split Phase 1 started is
complete. Ready to commit, open a PR, and expect the largest CI matrix
of the whole migration.

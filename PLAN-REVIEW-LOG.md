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

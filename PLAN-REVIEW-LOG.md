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

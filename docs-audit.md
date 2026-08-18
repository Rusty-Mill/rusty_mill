# docs-audit — rusty_yirp

Full-repo audit, 18 tracked docs (`README.md` + everything under `docs/`).
Ground truth built from: `Cargo.toml`/workspace manifests, `git ls-files`,
`sessionmgr --help` (live run of `target/debug/sessionmgr`), `.github/workflows/ci.yml`,
`grep` for `env::var` call sites, and `git log` per file. Not scoped — this is
the first docs-loop run on this repo.

## Findings

| Doc | Where | Claim | Classification | Ground truth | Fix | Size |
| --- | --- | --- | --- | --- | --- | --- |
| README.md | `Status`, L51-77 | "**Phase 2 complete**" — status section stops at Phase 2 | stale | `docs/phase-8-report.md` exists (8 phase reports total); PRs #24/#25/#26 merged to `main`; `sessionmgr --help` shows `fork`, `switch-agent`, `tui`, dependent-session flags all live | Rewrite the Status section to reflect current state (all 8 phases), or replace the phase-by-phase narrative with a link to the phase reports plus a short current-capabilities summary | M |
| README.md | `Status`, L78-79 | "Still outstanding on Windows: the Defender smoke test, and wiring the `longPathAware` manifest into the build via a `build.rs`" | stale | `docs/phase-2-windows-verification.md` documents the Defender smoke test as run and passed (24 concurrent sessions, closed as tested — PLAN.md risk item 7 itself says so); `crates/sessionmgr-daemon/build.rs` + its `embed-manifest` build-dependency embed the manifest, confirmed present in `git ls-files` | Delete this "still outstanding" line — both items are done | S |
| README.md | whole file | — | missing | `crates/sessionmgr-desktop/` (Tauri GUI, second shipped binary) has zero mentions anywhere in README — not in the architecture paragraph, not in Status, not in Building | Add a short paragraph/section: a second binary exists (`sessionmgr-desktop`), what it is, link to `docs/phase-8-report.md` | M |
| README.md | `Building`, L93-95 | "Gated agent-CLI adapter tests ... opted into per CLI (e.g. `SESSIONMGR_TEST_CLAUDE_CODE=1 cargo test`)" | stale | `grep -rn "SESSIONMGR_TEST_" crates` → zero matches anywhere in source. The actual mechanism (`crates/sessionmgr-daemon/tests/agent_needs_input_{claude,codex,gemini}.rs`) is a PATH probe — `Command::new("claude").arg("--version")` — that `eprintln!`s and returns early when the CLI isn't installed, no env var involved | Rewrite the sentence to describe the real PATH-probe skip, not an env-var opt-in | S |
| docs/plan/PLAN.md | `Verification`, L180 | "gated adapter tests run separately (`SESSIONMGR_TEST_CLAUDE_CODE=1 cargo test`, etc.)" | stale | Same ground truth as the README row above — same wrong claim, second location | Same fix, second location | S |
| docs/phase-5-report.md | L78 | `[sessionmgr_core::parent_readiness](../../crates/sessionmgr-core/src/dependency.rs)` | stale (broken link) | Doc lives at `docs/phase-5-report.md`; one `../` already reaches the repo root, so `../../` overshoots it. `check_references.py` confirms: `broken link docs/phase-5-report.md:78`. File itself exists at `crates/sessionmgr-core/src/dependency.rs` | Drop one `../`: `../crates/sessionmgr-core/src/dependency.rs` | S |
| docs/plan/PLAN.md | `Workspace structure`, L26-60 | Tree diagram lists `sessionmgr-core/protocol/git/proc/agents/daemon/tui` only | missing | `crates/sessionmgr-desktop/src-tauri` is a real workspace member (root `Cargo.toml` `[workspace] members`), shipping an entire second binary — absent from the diagram | Append a short entry for `sessionmgr-desktop/` (this doc already uses append-style status notes elsewhere, e.g. risk items 6/7 — same pattern fits), pointing at `docs/phase-8-report.md` | S |
| docs/plan/PLAN.md | `Phased milestones`, L148-158 | "Phase 6+ (gated on a dedicated research spike): switch-agent-mid-session and Fork together" framed as future/blocked; no Phase 7/8 entries exist | stale | `docs/phase-6-report.md` (fork, Claude) and `docs/phase-7-report.md` (switch-agent, all three CLIs) are both shipped and merged; Codex/Gemini fork support was added afterward too (per this session's own merged PRs). The gate this bullet describes has been resolved for two phases | Append short status notes to the Phase 6+ bullet, same append convention as risk items 6/7 in this doc; optionally one-line Phase 7/Phase 8 additions | M |
| docs/plan/CAPABILITIES.md | `Not yet in scope`, L392-410 | Lists Fork session, Switch agent mid-session, Dependent/chained sessions, and Grid/multi-pane layout as "not yet in scope, worth a deliberate decision" | stale | All four have shipped: `docs/phase-5-report.md` (dependent sessions), `docs/phase-6-report.md` (fork), `docs/phase-7-report.md` (switch-agent), and both `sessionmgr-tui::grid` and the desktop app's own grid (ported from it) exist in `git ls-files` | Append a status note per bullet (this doc is a fixed feature-target list from research, not a living status page — an append fits its own voice better than a rewrite), each pointing at the phase report that shipped it | M |
| docs/phase-8-report.md | whole file | Documents the original dark/minimal desktop UI and closes issue #23; last touched at commit `334d2dc` | missing | Two merged PRs landed after that commit and are undocumented anywhere: `50a931f` (light-theme, project-grouped-card redesign) and `44666fb` (breadcrumb view-toggle/Stop/diff/agent-pill/elapsed/kbd-badge controls) — both real, both live-verified under Xvfb at the time, neither has a line in any tracked doc | Judgment call, not mechanical: either an addendum section in `phase-8-report.md`, or a new `phase-8b-report.md` matching this repo's existing `-b` sub-report convention (`phase-3b`, `phase-4b`) | L |
| docs/decisions/0001, 0002 · docs/phase-1/2/3/4b/7-report.md | inline mentions of `.claude/settings.json` | — | false-positive (script only) | These describe files a session's own worktree writes at runtime, not paths in this repo's own tree; `check_references.py` flags them because an empty session-local `.claude/` directory happens to exist in this checkout right now. Not a doc defect | none | — |
| docs/phase-2-report.md · docs/plan/PLAN.md | inline mentions of `.git/index.lock` | — | false-positive (script only) | Same reason — a transient runtime lock file the doc is describing behavior around, not a path this repo ships | none | — |
| docs/decisions/0003-resume-fork-spike.md | L220 | `` `docs/en/sessions#where-transcripts-are-stored` `` | unverifiable | External Claude Code documentation citation shorthand, not a path in this repo; nothing in this tree can confirm or refute an external doc's URL fragment | none | — |
| docs/plan/SCOPE.md | whole file | Origin/competitive-landscape doc, explicitly marked superseded by PLAN.md where the two conflict | accurate | Self-aware framing already correct (README's own Governing Documents table states the supersession); Job-Object claims already flagged inside PLAN.md's "Corrected facts" section | none | — |
| README.md | `Clean-room boundary`, `Why this exists` | Clean-room sourcing claims, competitive differentiation claims | accurate | Unaffected by any later phase — same facts hold (Xirp/Conductor/Solo landscape hasn't changed and no code was added from those doc's sourcing) | none | — |
| .github/workflows/ci.yml comment block | top of file | Describes Windows-native focus, the three real bugs the matrix was built to catch | accurate | Matches the actual job matrix (`windows-latest`, `ubuntu-latest`, MSRV on `ubuntu-latest`) and step list (fmt, clippy, build, test) | none | — |

## Counts

| Classification | Count |
| --- | --- |
| stale | 6 |
| missing | 3 |
| accurate | 3 |
| unverifiable | 1 |
| false-positive (script only, no doc action) | 2 rows (covering 6 inline mentions across 6 docs) |

## Auto-eligible under `LOOP_HARNESS_MODE=auto`

Not applicable this run — `LOOP_HARNESS_MODE` is unset, so every row above
waits for explicit pick, per interactive mode. For reference, if it had been
set: the `docs/phase-5-report.md` broken-link row (mechanically verified by
`check_references.py`) would have been auto-eligible; every other row is
either a judgment call about scope/framing (PLAN.md, CAPABILITIES.md, README
status rewrites) or a content-creation decision (`phase-8-report.md`
addendum), and those always wait regardless of harness mode.

## Rows where the code is the suspect party

None. Every stale/missing row above is the docs falling behind already-
shipped, already-tested code — not a case where the documented behavior looks
like the *intended* one and the code is what's wrong.

## Scope

Whole-repo, all 18 tracked `*.md` files, not scoped to a date range or PR set.
Doc-comments (`///`/`//!`) were **not** audited — out of scope unless
explicitly requested (per docs-loop step 0).

# Goldfish Review Report: #598 build plan

**Target:** `PLAN.md` (lines 1-56), checked against the repository and DECISIONS D59, D62, D67.

**How this was run:** I had no subagent tool in this session, so no separate reviewers were spawned. I did all four passes myself (design, implementation readiness, codebase details, zoom-out/simplicity) and merged the results. Each finding names its pass.

Count: 4 blocking, 11 significant, 10 minor.

---

## Blocking

### B1. The baseline pin only applies when `protected` is set. The plan says it is "already delivered" for every candidate.
- **Pass:** codebase details
- **Where:** PLAN "Already delivered" ("`ProfileSet.baseline` pins every candidate's base") and PR 2. Code: `crates/apps/rusty_bbp_host/src/mcp.rs:825-854` (the guard returns `None` at :828 when `set.protected.is_empty()`, so `off_baseline` never runs). `profiles.rs:135-147` (the full-commit check on `baseline` also runs only when there are protected paths). `protect.rs:342-353`.
- **Problem:** For a `bbp open --base` task with no `protected` globs, candidates can build on any base. Then `base_test_changed` ("exists at base") has no fixed base to check against.
- **Fix:** In PR 2, check `off_baseline` whenever `baseline` is `Some`, not only when there are protected paths. Validate `baseline` as a full commit id on its own, outside the protected check. Give the "build on the baseline" refusal its own code; it currently reuses `ProtectedPath`. Correct the "Already delivered" line.

### B2. "Acted on" is defined differently from the code and the C-R9 contract
- **Pass:** design, codebase details
- **Where:** PR 1, affected-claims list ("accepted and acted on (a mark exists in the current epoch)"). Code: `crates/libs/protocol/rusty_bbp/src/claims.rs:356-361` (`acted_on`: any mark not `aborted`, in any epoch). Used in `claims_engine.rs:728, 857-859, 935, 1040, 1059`. Pinned by CL-51(b).
- **Problem:** Here is the case where the two rules disagree:
  1. A claim is marked as acted on in epoch 1.
  2. A human reopens it.
  3. It is re-accepted in epoch 2, with no new mark.

  The plan would reopen it as `by: intent`. Every other path (evidence, dep, supersede) escalates it, because it was acted on. That breaks the "an acted-on claim escalates instead of reopening" rule in D67.
- **Fix:** Use `ClaimRecord::acted_on()` as it is. Remove "in the current epoch". Add a trace test for exactly this case.

### B3. New inbox reasons break the Mill Board inbox, and the plan never touches it
- **Pass:** zoom-out, codebase details
- **Where:** PR 1 Host ("Inbox shows `task_paused` … and `intent_changed`"). Code: `crates/apps/rusty_bbp_host/src/inbox.rs:23-47` (`Reason`, `Reason::of`). `crates/apps/rusty_mill_board/src/inbox.rs:18-56` (strict serde `Deserialize` with no fallback variant), `:394` (`plan` dispatch), `:412` (the default Approve arm becomes `claim accept`).
- **Problem:** Adding `Reason::IntentChanged` or `Reason::TaskPaused` to the host's `bbp inbox` JSON makes Mill Board's whole inbox fail to deserialize as soon as one such escalation exists.
- **Fix:** Add `crates/apps/rusty_mill_board/src/inbox.rs` to PR 1 (CLAUDE.md routes #595 inbox work through `let-em-loose-expert`). Add both variants, with labels and `plan` arms:
  - TaskPaused: Approve is `resume open` only, with no totals.
  - IntentChanged: Approve is `claim accept` (if that is intended), Reject is `drop`.
  - Add a contract test that feeds the new host JSON to Mill Board.

### B4. `--claim ID... --domain D...` cannot be parsed: repeated flags overwrite each other
- **Pass:** codebase details
- **Where:** PR 1 Host (`bbp human intent --brief FILE [--claim ID]... [--domain D]...`). Code: `crates/apps/rusty_bbp_host/src/args.rs:8, 18` (`flags: HashMap<String,String>`; `insert` keeps only the last value).
- **Problem:** Only the last `--claim` or `--domain` would survive. That writes a narrower `affects` than Nano stated into the append-only log.
- **Fix:** Take comma lists (`--claims msg:1,msg:2 --domains http,db`), as `bbp inbox --task T[,T2...]` already does, or positional arguments. Refuse an empty item.

---

## Significant

### S1. `task_paused` must go through `transition`, and the host inbox only looks at `Escalated`
- **Pass:** implementation readiness
- **Where:** PR 1 `HostAction::TaskPaused`. Code: `engine.rs:1873-1888` (`escalate` emits an event, then `transition`). `engine.rs:1850-1852` (`transition` revokes live turns and emits `StateChanged`). `state.rs:921-924` (`escalated_from` is set only from `StateChanged`). `engine.rs:1504-1521` (`check_resume` allows `Open` only when `escalated_from == Some(Open)`). Host `inbox.rs:144, 178-200` (`task_at` is set only by `Event::Escalated`).
- **Problem:**
  - If `task_paused` changes the state inside the fold, `resumed(open)` is refused.
  - The inbox shows no pause at all.
  - If the task hit a task total earlier and was resumed, the inbox shows a stale `TaskTotal` with empty totals.
- **Fix:** Emit `TaskPaused`, then `transition(w, State::Escalated, now)`. Track the latest of `Escalated` and `TaskPaused` for `task_at`. Give TaskPaused its own `Reason` carrying `reason` and `detail`. Add a trace test: pause, `resumed(open)`, then an agent write is accepted.

### S2. A ceiling pause has no way out, so the task re-pauses straight away
- **Pass:** zoom-out
- **Where:** PR 1 pause, plus "the human resumes with the existing `resumed(open)`". Code: `orch-lens/src/meter.rs:146-155` (`reserve` keeps returning `TaskCalls` or `TaskSpend` while the account is over the ceiling; there is no per-task raise).
- **Problem:** After `resumed(open)`, orch's next call fails `Stop::TaskCalls` again and writes another pause.
- **Fix:** Pick and document one path:
  - (a) The inbox item names the config edit (raise `ceilings.task_calls` or `task_usd`) to make before resuming, and orch pauses again only if the ceiling value is unchanged since the last pause. `detail` carries the ceiling value.
  - (b) A human per-task allowance. This would be a new schema item, so it needs approval.

### S3. PR 2 changes the protocol, but the plan doesn't say so, and two refusal codes are missing
- **Pass:** implementation readiness
- **Where:** PR 2 (`baseline_red`, `base_test_changed`, `task_opened.baseline_digest`), plus "PR 1 first (protocol the others use)". Code: `command.rs:254-345` (`Code` has neither code; #597's host codes `ProtectedPath` and `BadGolden` were added there). `command.rs:70-98` (`OpenTask`). `event.rs:100-131` (`TaskOpened`; the optional fields use `#[rusty_serde(default, skip_serializing_if = "Option::is_none")]`).
- **Fix:**
  - Add `Code::BaselineRed` and `Code::BaseTestChanged`.
  - Add `baseline_digest` to both `OpenTask` and `TaskOpened`, with default and skip-if-none on each.
  - Add schema list item 13 entries, plus CL rows and a byte-identical golden (no `--base` gives the same log).
  - Either move these protocol pieces into PR 1, or say PR 2 touches `rusty_bbp` and the addendum.

### S4. The filtered view leaks through git, and its diffs would delete sensitive files
- **Pass:** design
- **Where:** PR 3, "Filtered view". Code: `crates/apps/rusty_bbp_host/src/confine.rs:57-62, 126-128` (the checkout is a `git worktree` that shares the repository's object store, which the harness may read).
- **Problem:**
  - A worktree with sensitive paths removed still exposes them, for example through `git show HEAD:<path>`.
  - The removed files show up as deletions in the builder's diff.
- **Fix:**
  - Make the view a plain file copy with no `.git` and no object-store read root.
  - Build the candidate diff against a snapshot of the same filtered tree.
  - Refuse any cloud-builder diff that touches `sensitive_paths`.
  - Add a host test: in the view, `git cat-file` of a sensitive blob fails.

### S5. Builder checkouts start at `HEAD`, not at the pinned baseline
- **Pass:** codebase details
- **Where:** PR 2 brownfield. Code: `confine.rs:127` (`repo.add_detached(&workspace, "HEAD")`).
- **Problem:** Once B1 is fixed, every builder candidate on a `--base` task whose `HEAD` differs from the baseline is refused for building off the baseline.
- **Fix:** Detach the worktree at `profiles.baseline` when it is set.

### S6. The brief pointer moves on `intent_revised`, so earlier `refine` answers fail
- **Pass:** design, zoom-out
- **Where:** PR 1, "store the new brief artifact (as `open` does)". Code: `state.rs:819-821` (`st.brief` follows the latest stored Brief artifact). `claims_engine.rs:466-469` (a C-R10 answer must ref `st.brief`, otherwise `refs_required`). `rusty_mill_board/src/inbox.rs:435, 468` (Refine steps use the brief from the escalation it read).
- **Problem:**
  - An answer or Refine built before a revision is refused afterwards.
  - The plan doesn't say how the inbox's Refine (an answer plus a human reopen) relates to `intent_revised`. C-R10 says #598 turns answers into intent versions.
- **Fix:**
  - State in C-R11 that the current brief is the latest brief, and that an answer refs the current brief.
  - Say in the plan whether Refine stays as it is or also offers "revise intent".
  - Add a CL test: an answer ref'ing the old brief after `intent_revised` gets `refs_required`.

### S7. `base_test_changed` interacts with #597 tests claims, and the inbox can't list the tests from the log alone
- **Pass:** design, implementation readiness
- **Where:** PR 2 `base_test_changed` and its inbox item. Code: `protect.rs:37-49` (the `tests`-domain exemption). `mcp.rs:848-853` (the static refusal lives in `Guard.refused`). Host `inbox.rs:122-157` (a pure log fold with one item per escalated claim).
- **Problems:**
  - (a) Changing an existing protected test through an accepted `tests` claim (the #597 path) is now refused unless that claim is `reversible: false`. State this, and the check order relative to `protected_path`.
  - (b) The inbox can't recompute which tests a candidate modifies without git. List them from the candidate's report `tests_changed` entries whose change is not `add`, filtered to paths that exist at the baseline.
  - (c) The Irreversible escalation and the human decision both act on the claim, not a candidate. With "one item per candidate (Q2)", define the item id (`<task>/<claim>/<candidate>/<seq>`) and what Approve does on an item that isn't the current candidate (recommendation: show it, but only the current candidate's item accepts).

### S8. The breaker's window state has no home in the store
- **Pass:** implementation readiness, zoom-out
- **Where:** PR 3 `orch-lens::breaker`, observe mode, `rusty_orch lens breaker`. Code: `orch-store/src/meters.rs:26-34` (the meter keeps only cumulative `(task, spent, calls)`; layout changes need a new schema tag, :3-4, :76).
- **Problem:**
  - Window boundaries, settled claims per window, |Δscore| per window, and where observe mode logs a would-trip are all unspecified.
  - A new breaker table isn't in D62's approved orch-store list (item classes, spend, reservations).
- **Fix:** Derive the window from `MeterRecord` calls (window n covers calls [40n, 40n+40)), together with the BBP revisions recorded at each settle. Write would-trip lines to the runner's own log. Ask the expert before adding any table.

### S9. Order of affected claims and cascades is unspecified
- **Pass:** design
- **Where:** PR 1 "For every affected claim …". Code: `claims_engine.rs:1008-1065` (`changed` and `left_accepted` reopen or escalate dependents within the same transaction).
- **Problem:** When dep A and dependent B are both affected, B's cause (`Dep(A)` or `Intent`) depends on the order they're processed. An acted-on A that escalates `intent_changed` also reopens its unacted dependents.
- **Fix:** Process in ascending MsgId order and read each claim's status when you reach it (B is skipped if it is already `open`). Document both cascades. Add CL tests for each.

### S10. The baseline run has to finish before anything is written at open
- **Pass:** implementation readiness
- **Where:** PR 2 "Red: … Nothing is written". Code: `admin.rs:124` (`profiles::freeze` writes before dispatch), `admin.rs:199-202` (the holdout suite is staged before `open_mode`).
- **Fix:** Run the baseline in scratch space before `holdout::stage` and `profiles::freeze`. Then freeze the set with `baseline` included, and write `baseline/<task>.json` before dispatching. Add a test that a red baseline leaves no files.

### S11. The Intent lens has no output channel
- **Pass:** zoom-out
- **Where:** PR 3 "Intent lens kind: suggests `affects`". Code: `orch-lens/src/config.rs:327-349` (a lens may post only claims or evidence).
- **Problem:** A suggestion has to exist before Nano writes `intent_revised`, and it is neither a claim nor evidence.
- **Fix:** Run it host-side over the old brief, the draft brief and the claim lines. It prints suggested `--claims` and `--domains` for the CLI or inbox and posts nothing to BBP. Say where the draft brief lives.

---

## Minor

- **M1. Spec wording** (`docs/spec/bbp-claims-addendum.md`):
  - :240 (C-R2, "not `escalated` on a task total") and :291 ("only when a task total") need to admit `task_paused`. The code at `claims_engine.rs:73-78` already refuses any non-`Open` state.
  - Add `intent_changed` to :280 and `intent` to :283.
  - :262 says the brief is "human-only at open"; add `intent_revised`.
  - New trace ids start at CL-102 (the highest existing one is CL-101).
- **M2. Omitted `affects`.** D59 says an omitted `affects` means all. Give `affects` a `#[rusty_serde(default)]` with `All` as the default. Pin the wire shape to D67's `{all: true}` or `{claims, domains}`.
- **M3. Unknown domain.** D67 doesn't cover a domain that matches no claim. Say it is accepted and affects nothing; don't add a refusal outside D67.
- **M4. Baseline report path.** Use `sha256(task)` in it, as `profiles.rs:161-165` does, for injectivity. Define "canonical" (the exact serializer and that no timing fields are included) so `baseline_digest` can be reproduced.
- **M5. Module and command names.** D67's module name `shadow_run.rs` is missing from the plan. `bbp shadow` already exists (#596, `bin/bbp.rs:30`), so name the `shadow:` run command differently.
- **M6. Config parser keys** (`config.rs:106-110, 245-255`). The parser refuses unknown keys:
  - Add `sensitive_paths`, `real_data` and `breaker` to the allowed keys. A missing `breaker` should default to `observe`.
  - There is no lens `kind` key. Either say Survey and Intent are prompt-only lenses, or add the key.
  - `domain_classes` appears again in PR 3 but was already delivered.
- **M7. `bbp host pause`.** No `bbp host` subcommand exists yet (`bin/bbp.rs:3-34`). Make the new one take an explicit `--op`, so an orch retry replays the result (R15) instead of getting `wrong_state`.
- **M8. Item classes.** Take the item class as the max over all of a claim's domains. Evidence takes its claim's domains.
- **M9. "Judged against the new brief".** Lens contexts don't carry the brief text today. `rusty_orch/src/lens/bbp.rs:276-279` caches `card.brief` once per session. Either refresh it after `intent_revised`, or drop the claim.
- **M10. Ceiling wiring.** If `lens run` (#1467, #1479) merges before PR 1, it can't call `bbp host pause` yet. Fix the CLI and exit-code contract in PR 1 so the #594 thread can code against it.

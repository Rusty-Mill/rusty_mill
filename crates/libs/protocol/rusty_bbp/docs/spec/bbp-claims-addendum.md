# BBP claims addendum (draft, v0)

Nov 2, 2026 · **Status: approved by the owner on 2026-11-02, including the schema and API changes listed below. Second owner review (2026-11-02): order-dependent settlement accepted; settled claims keep taking evidence and executable evidence reopens them; candidate runs approved if debited from the claim budget and sandboxed. Third owner review (2026-11-02): reopen symmetry with a supersede exception; dependency collapse reopens unacted dependents; `sole_candidate` is a recorded fact, not an escalation; `source` artifacts for cited evidence; human reruns past budget escalate; overdue action marks escalate. Q1 to Q5 decided. Fourth owner review (2026-11-02): Q4 accepted with a stricter accept line (12) for high-stakes and irreversible claims and −8 for a failing test; superseding open claims allowed; a human `choose` among candidates. Fifth owner decision (2026-11-02): a failing run of the current candidate blocks acceptance until a passing run that includes the failed profiles; interim rule that changed test code clears a block and counts only once another principal endorses it. Sixth owner decision (2026-11-02): added test files count too; adding tests needs endorsement only on a candidate that is clearing a block. Spec only; the engine change is a separate PR.**
Extends [`bbp-v0.5-implementation-spec.md`](bbp-v0.5-implementation-spec.md). Card: Mill Board #593 (Let Em Loose).

## Summary

A task may open in **claims mode**. Its agents post **claims** (small propositions) and **evidence** for or against them; the board, not the agents, computes each claim's score and settles it: accepted, dropped, void (a dependency fell) or escalated to the human. Evidence and claims are append-only and need **no turn**, so any number of agents post concurrently. Everything that changes state (a claim settling, an escalation, a human decision, a budget extension) stays serialized through the single transactional writer, as today.

Claims mode sits **beside** the stage lifecycle. A claims-mode task has no `planning … merge_gate` states. A task opened without `mode` is a stage task, byte for byte as before.

Owner decisions this rests on (2026-11-02): evidence posts need no turn; the one-live-turn limit stays for state changes, budgets and approvals; claims sit beside the stage lifecycle in v0, chosen per task; existing tasks are unchanged. Roles, lenses, model choice and track records stay in orch.

## Where this starts from (main b8467139)

| Today | Claims mode |
| --- | --- |
| Task lifecycle = the factory pipeline (`planning` → `plan_gate` → `build` → `test` → `review` → `merge_gate`) | Task states `open`, `escalated`, `closed`, `cancelled`. Each claim has its own status |
| Every agent write needs a live turn token (R14) | `claim`, `evidence`, `read`, `get_artifact` and `put_artifact` of `diff`/`candidate`/`source` are turnless, authorised by a per-assignment session credential (C-R1) |
| `wake_set` wakes **roles** (Addressed/State/Artifact/Observer) and the moderator grants a turn | `claims_delta` tells a host which claims changed for a **subscription**; the host (orch) decides whom to run. BBP grants no agent turn in claims mode |
| Budgets: task totals (messages, bytes, reads, turns, iterations) | Same task totals, plus a per-claim evidence budget scaled by stakes and reversibility |
| Escalation: budget crossed, request deadline, runner error | Claim escalates when stuck, out of budget, or irreversible at the accept line |
| `verdict` needs a selected run's report (R3) | `executed` evidence must cite a runner `test_report`; it outweighs `cited`, which outweighs `opinion` |

## Mode and configuration

`task_opened` gains two optional fields, written only in claims mode:

```json
{
  "schema": 5,
  "mode": "claims",
  "claims": {
    "accept_at": 8, "accept_at_strict": 12, "drop_at": -8,
    "min_authors": 2,
    "settle_window": 6, "settle_delta": 2,
    "evidence_base": 8,
    "mark_timeout_secs": 3600
  }
}
```

- `mode` absent means `stages`: no field is written, the schema stays 2, 3 or 4, and the log is byte-identical to today.
- `mode: "claims"` writes **schema 5**. A reader older than this addendum refuses schema 5 rather than replaying it as a stage task.
- `claims` is validated at open: `drop_at < 0 < accept_at ≤ accept_at_strict`, `min_authors ≥ 1`, `settle_window ≥ 2`, `settle_delta ≥ 0`, `evidence_base ≥ 1`, `mark_timeout_secs ≥ 60`. A bad value refuses the open as `bad_claims_config`, naming the field. The values above are the defaults; they are judgment, not data, and are fixed before the first run.
- `max_live_turns` and `policy` are not used in claims mode, because no agent turn is granted. A claims-mode open that names either is refused as `bad_claims_config`.

**Roster in claims mode.** Same `Role` type, no new fields. `post` may contain the two new kinds `claim` and `evidence`. The requirement table (planning/build/test/review) does not apply. Instead, validation needs at least one role with `post ∋ claim` and at least one with `post ∋ evidence`, or the open fails as `Unservable(claims)`. `verdict_at`, `final_decision` and `consultable` are ignored.

## Records

Both records are **message kinds** with typed bodies, like `verdict` and `decision`. That reuses message ids, `refs`/`evidence` resolution (R4), size limits (R8), `op` replay (R15), visibility and `read`.

### `claim`

```json
{
  "kind": "claim", "to": ["*"], "refs": ["art:01HBRIEF"],
  "body": {
    "text": "One shared HTTP client covers all 7 call sites without behaviour change.",
    "domains": ["http", "dedup"],
    "deps": ["msg:12"],
    "answers": "msg:4",
    "supersedes": "msg:9",
    "stakes": "medium",
    "reversible": true
  }
}
```

| Field | Rule |
| --- | --- |
| `text` | 1 to 1,200 characters (R8) |
| `domains` | 1 to 8 tokens, `[a-z0-9_-]{1,32}`. Opaque to BBP; used only to match subscriptions |
| `deps` | Earlier `claim` messages on the task, at most 16. They must already exist, so the graph is acyclic by construction |
| `answers` | Optional earlier `claim` this one is a candidate answer to (an open question). Candidates of one question compete; BBP records how many there are (C-R7) |
| `supersedes` | Optional earlier claim X this one (Y) replaces, in any status, if X is not already superseded. **It takes effect only when Y is accepted** (by the update rule, `choose` or a human `decision`); until then X behaves normally, so a claim post alone cannot disable anything. If X is acted on, Y's accept line escalates `supersedes_acted` instead of accepting, and only a human `accept` lets it replace X. When it takes effect, X becomes `superseded` whatever its status (`claim_superseded(X, by: Y)`), in the same transaction: X can no longer be marked or satisfy a dependency, any pending escalation or held rerun on X is withdrawn, X's dependents are handled as for a dep leaving `accepted` (or a dropped dep, if X was not accepted), and X keeps its action marks. Eligibility is checked again at Y's acceptance commit: if X was superseded meanwhile by a competing replacement, Y is still accepted on its own evidence but its supersession lapses (`claim_accepted` carries `supersedes_lapsed: X`) and X is untouched; the `supersedes_acted` check also uses X's marks as they stand at that commit. From then on X stays closed: evidence on it is recorded only and never reopens it |
| `stakes` | `low`, `medium` or `high` |
| `reversible` | `false` means the board never accepts it alone (C-R6) |
| `refs` | At least one (R2), usually the brief or an artifact |

A claim starts `open` with score 0 (neutral).

### `evidence`

```json
{
  "kind": "evidence", "to": ["*"], "refs": ["msg:17", "art:01HDIFF#L40-L52"],
  "evidence": ["art:01HREP"],
  "body": { "claim": "msg:17", "direction": "support", "kind": "executed", "strength": 1, "note": "…" }
}
```

| Field | Rule |
| --- | --- |
| `claim` | An earlier `claim` on the task, and it must appear in `refs` |
| `direction` | `support` or `counter` |
| `kind` | `executed`: `evidence` cites a runner `test_report` (written by the runner principal, so an agent cannot fake one). `cited`: `evidence` cites at least one artifact (including a `source` artifact holding outside material) or message range. `opinion`: no `evidence` required |
| `strength` | 1, 2 or 3, the author's own rating, for `cited` and `opinion`. `executed` evidence must carry 1: a test result is not a matter of rating (`bad_claim` otherwise) |
| `note` | 0 to 1,200 characters |

Evidence on a settled claim is still admitted (see Evidence after settlement). There is no retraction: a correction is new evidence in the other direction.

## The update rule (deterministic, integer only)

The score of a claim is a fold over its evidence in log order. There are no floats and no clock, so replay reproduces it exactly.

A claim's evidence is split into **epochs**. The first epoch starts when the claim is posted; each reopen (by evidence or by a human) starts a new one. The update rule reads only the current epoch. The card shows every epoch.

```
weight(kind, direction) = executed support 4 | executed counter 8 | cited 2 | opinion 1
delta(e)      = sign(direction) × weight(kind, direction) × strength   # executed: +4 or −8; cited ±2…±6; opinion ±1…±3
accept(c)     = accept_at_strict if stakes = high or reversible = false, else accept_at
score(c)      = Σ delta(e) over scoring evidence in c's current epoch
authors(c, +) = distinct principals with scoring support in the current epoch
authors(c, −) = distinct principals with scoring counter in the current epoch
```

Evidence is **scoring** when it is admitted while the claim is `open`, or when it reopens the claim (it is then the first item of the new epoch). Evidence recorded on a settled claim without reopening it is **recorded**: stored, shown, charged, never scored.

**Confidence** is shown as `p = 1 / (1 + 3^(−score/4))`, rounded to whole percent. It is for people only; no rule reads it. With the defaults, `accept_at = 8` is exactly 90% (3² = 9, so 9 / 10), `accept_at_strict = 12` is 96% (27 / 28) and `drop_at = -8` exactly 10%. A failing test weighs twice a passing one (owner, 2026-11-02): objections are cheap to raise and expensive to ignore, and a flaky test is handled by a budgeted rerun, not by a lower weight.

After each scoring `evidence`, after each candidate debit on an `open` claim (Budget), and after each final report of an `open` claim's current candidate (Failing runs block), the board evaluates that claim in this order. A settled or escalated claim is never evaluated by these rules; it changes status only as Evidence after settlement and C-R8 say. The first rule that fires wins, and its event is generated in the same transaction (Processing model, phase 4):

1. **Accept line.** `score ≥ accept(c)`, `authors(c,+) ≥ min_authors`, at least one support item is not `opinion`, every dep is `accepted`, and `failing(c)` is empty (Failing runs block):
   - if `reversible: false`: `claim_escalated(reason: irreversible)` (C-R6);
   - otherwise `claim_accepted`. If it is a candidate whose question has fewer than two candidates, the event carries `sole_candidate: true` (C-R7). BBP does not act on that fact; orch decides what to do about it.
2. **Drop line.** `score ≤ drop_at`, `authors(c,−) ≥ min_authors`, and at least one counter item is not `opinion`: `claim_dropped`. Dropping an irreversible claim needs no human, because not doing something can be undone.
3. **Budget.** `spent(c)` reaches `budget(c)`: `claim_escalated(reason: budget)`.
4. **Settling.** The current epoch has at least `settle_window` scoring items, and `|score now − score settle_window items ago| ≤ settle_delta`: `claim_escalated(reason: stuck)`.

A claim that crosses the accept line while a dep is still `open` stays `open`. It is re-evaluated whenever a dep settles (C-R5).

### Failing runs block

A failing test cannot drop a claim alone (`min_authors`), but it **blocks acceptance** (owner decision, 2026-11-02). The block is about the code that would be accepted, so it follows runner reports, not evidence posts: a failed run blocks even if nobody posts counter evidence about it.

```
current(c) = the candidate naming c that was stored last (log order); none before the first
failed(r)  = names of the profiles in report r with exit_code ≠ 0 or failed > 0
failing(c) = set of profile names, empty when c is posted, updated by each final report r
             of a run of current(c) (as current at r's commit):
               r failed:  failing(c) ∪= failed(r)
               r passed:  failing(c) −= names of the profiles r lists
               r error:   no change
```

- **The clearing run must include what failed.** A passing report removes only the profiles it lists, so a passing run of a reduced suite clears nothing it did not run. Granularity is the runner profile: `Report` carries profile names and failure counts, not test ids. Per-test ids would be a `Report` schema change and are not proposed; a lens that wants a finer split defines finer profiles.
- **Profile definitions cannot move under a block.** The owner asked that a failed profile's definition be pinned so a clearing run uses the same one. Profiles are already defined where no agent can write: the host freezes the profile set (each profile's program and arguments) when the task opens, at `<dir>/profiles/<task>.json` on the host, outside every sandbox and outside the Agent API. Its SHA-256 is `profile_digest` in `task_opened` and never changes for the task. The runner refuses a set whose bytes differ from that digest, and R16 refuses any report whose `profile_digest` differs. So no epoch-level hash is needed: every report on the task, failing or clearing, ran the same definitions (CL-61). What a candidate *can* change is test code inside its own tree; the interim rule below covers that until #597's holdout suites, which live outside the tree, replace it.
- **Only the candidate that would be accepted matters.** Storing a new candidate makes it current and carries `failing(c)` over: it is not cleared by being newer, only by the new candidate's own passing run that lists the failed profiles. A report on a candidate that is no longer current (a replaced candidate's late or rerun failure) leaves `failing(c)` unchanged, so it cannot block.
- **Epochs do not reset it.** `failing(c)` is a fact about the current code, not about scoring, so a reopen keeps it. This is what makes a reopen by a failing re-check (CL-38) start blocked.
- **Overrides.** A human `decision accept` or `choose` (C-R8) bypasses it like any other threshold. The card shows `failing(c)` and the report that set each entry.
- **Interim rule: changed tests clear and count only once endorsed** (owner, 2026-11-02; replaced by #597 holdouts). The frozen profile set gains `test_paths` (globs, required in claims mode, covered by `profile_digest`; `bad_claims_config(test_paths)` otherwise). After applying a candidate's diffs the runner lists in the report `tests_changed`: one entry `{path, change}` per path matching `test_paths` that the diffs add, modify or delete relative to `base`, with `change` one of `added`, `modified`, `deleted` (the engine needs the kind to apply the rules below). Traces write an entry as its path where the kind is obvious. Added files are listed too: an added file can weaken existing tests without touching them (a pytest `conftest.py` with an autouse fixture or collection hook that skips them, a Rust `tests/common/mod.rs` or a config file under `tests/` that changes how they build or run), and no file-level rule can tell a harmless addition from that (owner, 2026-11-02). Adding tests never needs endorsement unless that candidate is clearing a block. Then:
  - a `passed` report with non-empty `tests_changed` **does not clear** `failing(c)` until its candidate is endorsed (below). If the endorsement commits later, the clearing applies at the endorsement's commit, provided the candidate is still `current(c)` and the report is still its latest final report; the accept line is then evaluated as for any scoring evidence. Failures always count: a `failed` report updates `failing(c)` as usual;
  - `executed` support citing such a report is refused `tests_unendorsed` until the test change is **endorsed**, when `tests_changed` names a modified or deleted file, or names only added files while `failing(c)` is non-empty. Executed support citing a report whose `tests_changed` names only added files, on a claim with no block, is admitted as usual. The same endorsement unlocks both clearing and scoring. An endorsement is `cited` support on the same claim, citing that candidate, by a principal who authored neither the candidate nor any of its diffs. After that, executed support citing its reports is admitted and scores as usual. The endorsement is ordinary cited evidence (weight 2 × strength) and counts as its author's support;
  - the card shows `tests_changed` and the endorsement, if any.
  
  So a wrong test that really must change is cleared by a second principal's judgement, never by the builder who changed it alone. **Known limits:** `test_paths` sees files, so tests inside source files (Rust `#[cfg(test)]` modules) are not covered (accepted by the owner, 2026-11-02). Nor is a test-support or configuration file outside `test_paths` (a root `conftest.py`, `pytest.ini`, `build.rs`), so a profile's `test_paths` must list every path its runner reads tests, fixtures or test configuration from. #597 holdouts, which live outside the tree, cover both.
- **Holdout suites (#597, owner 2026-11-02).** A claims task may open with a second frozen profile set, the holdout suite, whose SHA-256 is `holdout_digest` in `task_opened` (optional; refused `bad_claims_config(holdout_digest)` in stage mode). Its tests live on a host path no sandbox mounts, so builders cannot read or edit them.
  - **Holdout candidate.** A `candidate` with `suite: holdout`. Only a role holding `holdout` (`write:holdout`) may store one (`kind_forbidden` otherwise); the task must have a holdout suite and the manifest's `base` and `diffs` must equal those of `current(c)` (`holdout_mismatch` otherwise). Its run is debited like any run. It never becomes `current(c)`.
  - **Reports bind to their suite.** The suite of a report is its candidate's, so `Report` carries no suite field. R16 checks a holdout report's digest against `holdout_digest` and a build report's against `profile_digest` (`profile_mismatch`). Holdout profile names start with `holdout:` or `golden:`, build profile names never do (`inconsistent_report`), so a build run can neither set nor clear a holdout failure.
  - **Blocking.** A final report of a holdout candidate whose manifest equals `current(c)`'s updates `failing(c)` exactly like a report of `current(c)`. A holdout run on replaced code changes nothing. An endorsement of `current(c)`'s test change also covers the holdout candidates building its code, whether it arrives before or after their reports.
  - **Accept line, one more condition (G1).** If the task has a holdout suite and `c` has a current candidate, acceptance also needs a `passed` holdout report on `current(c)`'s code, under the task's `holdout_digest`, stored after `current(c)`, whose `tree` equals that of `current(c)`'s latest report. A manifest may name its base by a ref that moves, so equal manifests do not prove equal checkouts. Claims without candidates are unaffected; a human `accept` or `choose` still overrides. The card's `holdout_due` says the claim waits for one, which is what orch schedules the holdout lens on.
  - **Evidence.** The holdout lens posts `executed` evidence citing its report with the usual weights; it counts as one more distinct author.
  - **On the host.** The holdout set is a profile set of `holdout:`/`golden:` profiles plus an absolute directory and the SHA-256 of every file in it, stored content-addressed at `<dir>/profiles/<sha256(task)>.holdout/<digest>.json` (`bbp open --mode claims ... --holdout-profiles FILE --holdout-root DIR`). A holdout run checks the directory against the manifest first (a changed, missing or planted file, or a symlink, makes the run `error`), then runs with the directory readable and named in `BBP_HOLDOUT`; build runs never get it. Its log artifact is a summary (profile, exit code, failures); the full output stays at `<dir>/holdout-logs/<sha256(task)>/run-<n>.log`, host-only. The MCP `put_artifact` of a candidate takes `suite: holdout`.
  - **Golden sets (#597 section 3).** `golden/<intent>/` holds `cases/<case>/case.toml` (`cmd`, `args` with `{golden}` for the set's directory, optional `stdin`, and `[expect]` with `exit`, one of `stdout`, `stdout_file` or `json_file`, and optional `stderr_contains`), fixtures, `MANIFEST` (`<sha256>  <path>` per file, absolute paths for host-only private fixtures, then `set <sha256 of those lines>`) and `SIGNOFF` (`digest`, `signer`, `date`). `bbp open --golden DIR` refuses `bad_golden` unless every file matches `MANIFEST` and `SIGNOFF` names the set digest; a signed set becomes the `golden:<intent>` profile of the holdout suite, run by `bbp golden-run DIR` in the candidate's tree, which re-checks the sign-off, compares exactly or as JSON, and prints verdicts only. A `*_file` must be a relative path inside its case directory that `MANIFEST` lists, verified on read; a holdout run is granted each private fixture file, not its directory.
  - **Suite versions (G3).** When the intent is refined, the holdout lens writes a new suite version. If it only adds files, the host writes `holdout_extended(digest)` (host action); a version that removes or changes an existing file needs the human's `holdout_updated(digest)`. The host makes that file-level check before writing; the core refuses either on a task without a holdout suite (`holdout_mismatch`) or for the current digest or any digest the task has retired (`duplicate`), since a retired suite's old passes would count again. After either, `holdout_digest` is the new one: reports under the old digest are `profile_mismatch`, every open claim with a candidate needs a fresh holdout pass, and holdout entries (`holdout:`, `golden:`) leave `failing(c)` because that fresh pass re-runs whatever the new suite keeps. Accepted claims stay accepted; orch schedules their re-check. Every claim with a candidate is marked changed at the version event, so `claims_delta` shows it.
- **Protected test paths (#597 G2: refuse, don't restore).** The frozen profile set may name `protected` globs (covered by `profile_digest`, not written when empty). When an agent stores a candidate, the host's Agent API reads each diff's paths as `git apply` does (both sides of a rename) and refuses `protected_path`, naming each path, if any diff adds, modifies or deletes a protected path; nothing is stored or debited. Exemptions: a candidate on a claim in the `tests` domain whose diffs touch only protected paths, and diffs of the current candidate of an accepted `tests` claim, which other candidates may then list. The core adds one accept condition: a `tests` claim with a candidate accepts only with scoring support from a principal who authored neither that candidate nor its diffs. The set must also name `baseline`, a full commit id: every candidate on the task builds on it (`protected_path` otherwise), so an older revision with weaker tests, which no diff would show, cannot be chosen as a base. Globs must be in the matcher's grammar (relative `/`-separated segments, none empty, `.` or `..`; `*`, `?`, and `**` as a whole segment; no `[`, `]`, `{`, `}`, `\`, `!`), checked at open (`bad_claims_config(protected)`), since a pattern read literally would protect nothing. The rule is judged against the state each dispatch attempt commits on, again after a revision conflict, and never against an op the core already committed, which replays its stored answer. With `protected` set, no build run carries an unaccepted test change, so `tests_changed` stays empty and `tests_unendorsed` never fires; the interim rule stays for tasks opened without `protected`.
- A blocked claim is still evaluated by the drop, budget and settling lines, so a block that nobody clears ends as a drop, a `budget` escalation or a `stuck` escalation, never a silent wait.

### Settlement follows committed order

The rule is evaluated per commit, in log order, against the claim as it stands at that commit. A settlement takes effect at the commit that causes it. Evidence committed after it is judged against the settled claim (Evidence after settlement), even if its author sent it before the settling post committed.

**The guarantee is that the same committed log always replays to the same statuses, epochs and scores.** Order independence is not guaranteed (owner decision, 2026-11-02): the same evidence committed in a different order can settle differently. With the defaults, on a low-stakes reversible claim, a passing test from `a` (+4), a passing test from `b` (+4) and a failing test from `c` (−8), each from its own run:
- in the order a, b, c, the claim is accepted at b; c then reopens it, and it is `open` in a new epoch at −8;
- in the order a, c, b, it is never settled and stays `open` at 0.

That is intended: the board decides on what it had when it decided. Nothing is lost silently. Every attempted post is either committed or refused with a code the author sees, and a refusal is logged as `rejected` like any other.

### Evidence after settlement

A settled claim keeps taking evidence. What that evidence does depends on its kind and on the settled status:

| Claim status | Executed evidence against the outcome | Any other evidence |
| --- | --- | --- |
| `accepted` | `counter` citing a `failed` report **reopens** it | Recorded |
| `dropped` | `support` citing a `passed` report **reopens** it | Recorded |
| `escalated`, `void` | Recorded (the human decides an escalation; a void claim waits for its dep) | Recorded |
| `superseded` | Recorded: a replaced claim stays closed | Recorded |

- "Executed" means a runner `test_report` bound to the claim (C-R4). Tests, holdout sets and golden sets are all runner profiles and all produce one. Cited and opinion evidence never reopen a claim and never move its confidence.
- **Reopen by evidence.** The board writes `claim_reopened(by: evidence)` in the same transaction. The claim is `open`, a new epoch starts, and that evidence is its first scoring item (so a counter reopen starts below zero). It must earn its way back to a settlement with fresh evidence from `min_authors` principals in the new epoch.
- **Reopening a claim that was acted on escalates instead.** A claim is *acted on* when it carries an action mark (below) that has no `aborted` result, in any epoch. Executed evidence against the outcome of such a claim writes `claim_escalated(reason: reopened_after_action)` instead of reopening it, and the claim goes to the inbox. The human then decides `reopen`, `accept` or `drop` (C-R8). Once acted on, a claim stays acted on: the action happened in the world, so every later evidence-driven reopen escalates too.

### Action marks (mark before acting)

The host records use of an accepted claim **before** it acts, so no race between an action and contrary evidence can erase the record:

1. **Mark.** `claim_acting(claim, epoch, ref)`, fenced on the epoch the host saw. Refused `wrong_state` unless the claim is `accepted` in exactly that epoch. **The host must not act unless the mark committed.** `ref` names the intended use (a PR, a merge).
2. **Act** outside BBP.
3. **Result.** `claim_action_result(mark, outcome)` with `done` or `aborted`. It is accepted whatever the claim's status is now, since the action already happened or did not. It binds to the mark, and through the mark to the mark's epoch, so a late receipt can never attach to a later epoch. A result for an unknown mark is `ref_unresolved`; a second result for one mark is `duplicate` (a retry with the same `op` returns the original result, R15).

Interleavings:
- Mark commits, then contrary executed evidence commits before the action finishes: the claim is acted on, so it escalates `reopened_after_action`. The receipt then lands on the mark whatever the outcome; `aborted` clears the mark, but the escalation stays for the human.
- Contrary evidence reopens the claim first: the mark is refused `wrong_state` and the host does not act.
- A host that crashes between mark and result leaves the mark open, which counts as acted on (conservative). After `mark_timeout_secs` it escalates `action_unconfirmed` (Overdue marks), and the human reconciles it with a `claim_action_result` from the human channel.
- **Dependents.** When an accepted claim leaves `accepted` (reopened, dropped by a human, or escalated after action), its accepted dependents are handled as in Dependencies: reopened if nothing acted on them, escalated `dep_collapsed` if something did. Open dependents stay open and stop meeting the accept line until the dep is accepted again.
- Every evidence item, recorded or scoring, counts toward `spent(c)`. At `spent(c) ≥ budget(c)` further evidence on the claim is refused `claim_budget` until the human records `budget_extended(claim)`.
- **Overdue marks.** A mark with no result `mark_timeout_secs` after it committed is reported by the moderator as `claim_escalated(reason: action_unconfirmed, mark)`, so a crash cannot leave a claim marked as acted on with nothing checked. The host's moderator checks this on its wake loop, using the `now` it already passes to the engine, so replay reproduces it from the logged event. The claim keeps its status; the inbox shows the escalation, and the human records `claim_action_result` (`done` or `aborted`).

**No rule reads who the author is beyond counting distinct principals.** Track-record weighting is out of v0 (open question Q2).

### Budget

```
base(c)   = evidence_base × stakes_factor × reversibility_factor
budget(c) = base(c) × (1 + number of budget_extended(c))
stakes_factor        = low 1 | medium 2 | high 4
reversibility_factor = reversible 1 | irreversible 2
spent(c)  = evidence items on c (all epochs) + runs started for candidates naming c
```

In plain terms: **one unit buys one evidence post or one test run.** With the defaults a claim gets 8 units (low stakes), 16 (medium) or 32 (high), doubled if irreversible, so 8 to 64. A human extension always adds exactly one `base(c)`, never an amount typed by hand, and an approval of an irreversible action adds none.

The defaults give 8 to 64 units per claim. One evidence item costs 1 and one run costs 1, so a run is debited from the claim it tests (owner decision, 2026-11-02). A candidate that would start a run on a claim with `spent(c) ≥ budget(c)` is refused `claim_budget` and no run starts (human reruns differ, see Runs and candidates). A debit on an `open` claim is followed by the update rule, so the debit that reaches the budget escalates it. A debit on an `accepted` or `dropped` claim (a re-check) changes nothing but `spent(c)`: it never re-runs the accept or drop line, never escalates, and only executed evidence from that run can reopen the claim. Task totals (`messages`, `bytes`, `reads`) still apply and are charged as today (R11), since `claim` and `evidence` are messages. The `turns` and `iterations` budgets are not used in claims mode.

### Dependencies

- When a claim becomes `dropped` or `void`, every claim that lists it in `deps` and is not yet settled becomes `void` (`claim_voided(by)`). This repeats to a fixed point in the same transaction, so a collapse is atomic.
- An already-`accepted` dependent of a dep that leaves `accepted` (dropped, voided or reopened) is **reopened automatically** if nothing acted on it: `claim_reopened(by: dep)`, a new epoch. It is then treated like any open dependent in the same transaction: voided if the dep was dropped or voided, waiting if the dep is open. A dependent that **was acted on** (Action marks) is escalated `dep_collapsed` instead, because undoing it is the human's call (owner decision Q3, 2026-11-02). The cascade repeats to a fixed point, so a reopen of a dependent can in turn reopen or escalate its own dependents.
- **Every reopen checks the budget.** After any reopen that is not a human `reopen` (those need `budget_extended` first, C-R8), the board runs the budget line on the reopened claim in the same transaction: a claim reopened with `spent(c) ≥ budget(c)` escalates `budget` at once, so it never sits open with no units to spend.
- When a dep becomes `accepted`, each `open` dependent is re-evaluated against the accept line.

## Rules (claims mode)

| Id | Rule | Rejects with |
| --- | --- | --- |
| C-R1 | In claims mode, `post` of `claim` or `evidence`, `read`, `get_artifact`, and `put_artifact` of `diff`, `candidate` or `source` need no turn. They carry the **session credential** issued to the principal at `assigned` and invalidated when the role is reassigned or the task ends. Credential binding replaces R14 for exactly these operations in claims mode. Any other Agent API mutation in claims mode is refused, and stage mode keeps R14 unchanged. | `bad_credential`, `wrong_mode` |
| C-R2 | The task is in `open` (not `escalated` on a task total, not terminal). A stage-mode task refuses both kinds. | `wrong_state`, `wrong_mode` |
| C-R3 | Body fields as in Records; `deps`, `answers`, `supersedes` and `claim` resolve to `claim` messages on the task (R4). A `supersedes` target that is already superseded, or that is among the new claim's own deps directly or transitively, is `bad_claim` (a replacement cannot rest on what it retires; deps never change after posting, so the check at admission is final). A new claim may not name a `dropped`, `void` or `superseded` dep, since it would be born dead; an `open`, `accepted` or `escalated` dep is fine. | `bad_claim`, `ref_unresolved`, `wrong_kind`, `dep_closed` |
| C-R4 | Evidence targets any claim on the task with `spent(c) < budget(c)`; whether it scores, reopens or is recorded follows Evidence after settlement. `executed` cites a final runner `test_report` of a run whose candidate names the same claim; `support` needs that report `passed`, `counter` needs it `failed` (an `error` report is not evidence). `cited` cites at least one artifact or message. Executed support citing a report with `tests_changed` needs an endorsement first, except a report that only adds test files on an unblocked claim (Failing runs block). | `claim_budget`, `evidence_required`, `evidence_unbound`, `not_passed`, `tests_unendorsed` |
| C-R5 | Settling is generated by the board (update rule), never posted by an agent. | n/a |
| C-R6 | An irreversible claim is never accepted without a human `decision`. | n/a (escalates) |
| C-R7 | When a candidate claim (`answers` set) is accepted while its question has fewer than two candidates, `claim_accepted` records `sole_candidate: true` and `claim_card` shows it. BBP takes no other action (owner, Q5). | n/a |
| C-R8 | A human `decision` with `subject` a claim and `outcome` `accept`, `drop` or `reopen` settles or reopens it (a reopen starts a new epoch). `outcome: choose` with `subject` a question and `chosen` one of its candidates accepts the chosen candidate and drops every other candidate of that question that is not already `dropped`, `void` or superseded, in one transaction; dependents follow Dependencies, and a dropped candidate that was acted on is dropped all the same (the human chose) and keeps its action record. `reopen` on a budget escalation needs a `budget_extended(claim)` first, which adds exactly one `base(c)`. Human decisions bypass `min_authors` and thresholds; they are the override. | `stale_rev`, `wrong_state` |
| C-R9 | `claim_acting(claim, epoch, ref)` is written by the host or the human only on a claim `accepted` in that epoch, before the action. `claim_action_result(mark, done \| aborted)` names an existing mark, at most once (Action marks). | `wrong_state`, `ref_unresolved`, `duplicate` |

R1 to R13, R15 and R18 apply unchanged, except that R11's per-turn read charges become per (principal, artifact id) in claims mode and `reads_per_turn` does not apply. R17 never fires in claims mode (no gates). R16 is replaced in claims mode by the gating rows below; in stage mode it is unchanged.

## Runs and candidates (claims mode)

This is the path from a prototype to `executed` evidence, using the normal APIs.

| Kind | Writer | Credential | Rule |
| --- | --- | --- | --- |
| `diff` | role with `write` ∋ `diff` | session credential | Envelope limit. No state gate (claims mode has no `build`) |
| `candidate` | role with `write` ∋ `candidate` | session credential | Manifest `{base, diffs, claim}`. `claim` is **required** in claims mode and **refused** in stage mode, so stage gating is unchanged. `claim` must be an `open`, `accepted` or `dropped` claim on the task (a candidate on a settled claim is a re-check, for example a holdout run against an accepted claim), with `spent(c) < budget(c)` (`claim_not_open`, `claim_budget`); every `diffs` entry must be a `diff` on the task (R4). Storing it debits one unit from the claim (Budget) |
| `test_report`, `log` | runner | run secret | As R16: names its run and that run's candidate, presents the run's secret, is the first final report or log of that run, and `passed` needs every profile clean with `profile_digest` equal to the task's |
| `source` | role with `write` ∋ `source` | session credential | Outside material for `cited` evidence: `{origin, retrieved_at, excerpt, digest}`, envelope-limited, `excerpt` at most 4,000 characters. Turnless in claims mode, refused in stage mode (`wrong_mode`). BBP checks shape and digest only, not truth: a `source` is still `cited` (weight 2), never `executed` |
| `spec`, `brief` | no agent | | `spec` is refused in claims mode (`wrong_mode`). `brief` is human-only at open, as now |

- **Each stored candidate gets its own run.** In the same transaction the moderator records `run_started(candidate, run)` and hands the supervisor the run id and secret out of band, as today. Several candidates may have pending runs at once; how many execute together is the host's admission slots, not a protocol limit. A candidate has at most one live run. A human `rerun(candidate)` revokes it and selects a fresh one (T12's effect, no state change), and the old secret is dead, so a late report is `stale_run`.
- **Every run start is debited, in the same transaction.** Storing a candidate and a human `rerun` both start a run, so both check `spent(c) < budget(c)` and debit one unit atomically with `run_started`. At `spent(c) ≥ budget(c)`:
  - storing a candidate (the only run start an agent has) is refused `claim_budget`, and nothing is written;
  - a human `rerun(K)` is held instead of refused: the moderator writes `rerun_held(claim, candidate)` and the inbox shows it as a budget escalation. The claim's status does not change. Approve = `budget_extended(claim)` (one `base(c)`) and then the rerun, in one transaction; Reject withdraws it. A second rerun of the same candidate while one is held returns the held one.
  - In every case the old run stays live with its secret valid until a new run actually starts. A replay of the same `op` returns the original result and debits nothing (R15). There is no other run-start path.
- **Sandbox only.** A claims-mode run executes only inside the host's sandbox (`rusty_sandbox`, through `rusty_worktree`), never on the host directly. This is a host requirement, since BBP cannot see where a run executes: a host that cannot sandbox refuses to open a claims-mode task (`bad_claims_config(sandbox)`), and the runner principal is the only writer of reports.
- **Reads.** `get_artifact` needs the session credential and the role's `read_artifacts`, as in stage mode.
- **Binding.** An `executed` evidence item binds to one final report, and through it to one run, one candidate and one claim (C-R4). Evidence citing a report from a different claim's candidate, a non-final report or a report the runner did not write is `evidence_unbound`.

## Events (new)

| Event | Writer | Payload |
| --- | --- | --- |
| `claim_accepted` | moderator | claim, score, evidence count, `sole_candidate` when true, `supersedes_lapsed` when a competing replacement got there first |
| `claim_dropped` | moderator | claim, score, evidence count |
| `claim_voided` | moderator | claim, by (the dep that fell) |
| `claim_escalated` | moderator | claim, reason: `irreversible`, `supersedes_acted`, `budget`, `stuck`, `dep_collapsed`, `reopened_after_action`, `action_unconfirmed` (with the mark) |
| `rerun_held` | moderator | claim, candidate (a human rerun waiting on budget) |
| `claim_superseded` | moderator | claim, by (the superseding claim), prior status; written when the superseding claim is accepted |
| `claim_reopened` | human or moderator | claim, by (`decision`, the reopening evidence, or the dep that left `accepted`), new epoch number; a human reopen also carries the expected `rev`. Reopening a claim does not revive dependents it voided; each needs its own `reopen` |
| `claim_acting` | host or human | claim, epoch, ref (C-R9) |
| `claim_action_result` | host or human | mark, `done` or `aborted` (C-R9) |
| `run_started` | moderator | candidate id, run id (existing event, also emitted in claims mode on candidate storage) |
| `credential_issued` | moderator | role, principal, credential digest (the secret itself is never logged) |

Task-level state in claims mode:
- A claim escalation does **not** change the task state. Only that claim waits for the human; turnless posting on other claims continues. The inbox (#595) reads `claim_escalated` events.
- The task goes `escalated` only when a task total (`messages`, `bytes`, `reads`) is crossed, as T3 does today. Then every agent write is refused until the human records `budget_extended` and `resumed(open)`.
- The task ends only by a human `close` or `cancelled`. The board never closes it.

## Reads, delta and subscriptions

- `read(after, limit)` returns messages as today, including `claim` and `evidence`. In claims mode it is authorised by the session credential and charged per call (R11).
- `claims_delta(after_rev, domains)` is a pure function over `TaskState`: the claims whose status, score or evidence changed after `after_rev`, restricted to claims with a domain in `domains`, plus any claim that depends on one of those. It is what a host's subscription polls or wakes on (the host's push waker already watches the log). It replaces `wake_set` for claims mode and names no role.
- **Visibility.** `claims_delta`, `claim_card` and `claim_line` show claim text and evidence, so they follow the same rule as `read`. On the Agent API they need the session credential and return only claims and evidence the caller's `read_messages` allows. A `scoped` role sees a claim only if the human addressed it to that role, and it never sees other agents' evidence. A claim it may not read is not returned, not counted, and cannot be fetched by id. The filter applies to nested content too: inside a returned card, evidence items, dependencies, candidates (`answers`) and candidate artifacts the caller may not read are omitted, and a `claim_line` or `claims_delta` entry for a hidden dependency is omitted rather than summarised. Artifact details also need `read_artifacts`. Stage-mode visibility is unchanged. The unfiltered views are on the human channel and the host side only (the moderator, and orch as the host that assembles lens contexts), never on the Agent API.
- `claim_card(claim)` and `claim_line(claim)` are free (no charge) and are the human-readable views:
  - card: text, status, score, confidence, evidence list with direction/kind/strength/author, deps, candidates;
  - line, for compaction: `msg:17 ACCEPTED +12 (90%) 5 evd · One shared HTTP client covers all 7 call sites…`. orch uses the line form for settled claims in a lens's context.

## Concurrency

All writes still go through the single transactional writer: `FsStore` takes an exclusive lock per append and refuses a stale revision, and the `Driver` reloads and recomputes on conflict (TR-29). Two agents posting at the same time are serialized into some commit order, and each post is judged against the state at its commit (Settlement follows committed order). If the first one settles the claim, the second is judged on recompute against the settled claim: recorded, or a reopen if it is executed evidence against the outcome. The control plane (claim settling, escalation, human decisions, budget extensions, run selection) is serialized by that same writer, so the one-writer guarantee for state changes holds with no turn. No CRDT is needed.

## Schema and API changes (need approval before implementing)

1. **Log schema 5** for claims-mode tasks. Stage tasks are unchanged (schemas 2 to 4, byte-identical).
2. **`MessageKind` gains `Claim` and `Evidence`.** This is a public enum in `rusty_bbp`. Exhaustive matches in `rusty_bbp_host` and `orch-bbp` must handle them, which is a cross-crate API change.
3. **`State` gains `Open`** (claims mode only), or claims mode keeps a separate `ClaimsState`. I prefer a separate enum, so stage-mode matches stay exhaustive and untouched. This is an implementation detail for the PR.
4. **Agent API (MCP):** `post` accepts the two kinds with a session credential; new tools `claims_delta`, `claim_card`, `claim_line`, credentialed and filtered by `read_messages`. The human channel adds `decision` on a claim, `reopen` and `budget_extended(claim)`.
5. **Session credential** is a new host requirement: one per assignment, injected by the adapter, never shown to the model (same treatment as turn tokens).
6. **Candidate manifest** gains an optional `claim` field: required in claims mode, refused in stage mode.
7. **Action marks** (`claim_acting`, `claim_action_result`) are new host/human writes, and the host must mark before acting, and **sandboxed runs** are a host requirement for claims mode (orch-bbp runs candidates through `rusty_sandbox` only).
8. **Third review additions:** the `supersedes` claim field, the `source` artifact kind, the `rerun_held` event, the `action_unconfirmed` escalation and `mark_timeout_secs`, and `sole_candidate` moved from an escalation reason to a field on `claim_accepted`.
9. **Fourth review additions:** `accept_at_strict`, executed counter weight 8 with executed strength fixed at 1, `claim_superseded` and the `superseded` status, and the human `choose` decision.
10. **Fifth decision:** the claim's `current` candidate and `failing` profile set in claims-mode state and on `claim_card`. Derived by the fold from existing reports; no new record or event.
11. **Interim test-change rule:** `test_paths` in the host profile set (claims mode), `tests_changed: [{path, change}]` on `Report`, and the `tests_unendorsed` code. Removed or relaxed when #597 holdouts land.
12. **Holdout suites (#597, approved 2026-11-02):** `holdout_digest` on `open` and `task_opened`, `suite` on the candidate manifest (default `build`, not written), the role capability `holdout`, the `holdout_mismatch` code, `holdout_due` on `claim_card`, the host action and event `holdout_extended(digest)` and the human action and event `holdout_updated(digest)`. Host: `protected` globs in the profile set and the `protected_path` code; core: the `tests` domain's accept condition. All optional and not written at their defaults, so existing logs are byte-identical.

No change to `rusty_crdt`, `coedit` or the stage-mode rules.

## Trace tests (to write before the engine change)

Named `tests/claims.rs::cl_NN_*`. Each must fail against a core with the corresponding rule removed. Builders: two lens principals `a`, `b`, a third `c`, the runner, the human.

| Id | Sequence | Expected |
| --- | --- | --- |
| CL-01 | Open a stage task with no `mode` | Log byte-identical to a pre-addendum build's (golden) |
| CL-02 | Open claims mode with `accept_at = 0` | `bad_claims_config(accept_at)`, nothing written |
| CL-03 | Claims roster with no role posting `evidence` | `Unservable(claims)` |
| CL-04 | `a` posts evidence with no turn and a valid credential | Accepted. No `turn_granted` exists in the log |
| CL-05 | `a` posts evidence with `b`'s credential, or after reassignment | `bad_credential` |
| CL-06 | `claim` posted to a stage-mode task | `wrong_mode` |
| CL-07 | `a` and `b` each post executed support (score 8, 2 authors), low-stakes reversible claim | `claim_accepted` in the transaction of the second post |
| CL-08 | `a` alone posts executed support three times (score 12, 1 author); separately, executed evidence with strength 2 | Stays `open` (`min_authors`); `bad_claim` |
| CL-09 | `a`, `b` post opinion support until score ≥ 8 | Stays `open` (no non-opinion support) |
| CL-10 | `a` posts one executed counter (−8, 1 author), then `b` posts one | Stays `open` after `a` (`min_authors`); `claim_dropped` at −16 after `b` |
| CL-11 | `executed` evidence citing a `test_report` the runner did not write, or citing a `diff` | `evidence_unbound` / `wrong_kind` |
| CL-12 | Irreversible claim reaches the accept line; another claim keeps taking evidence meanwhile | `claim_escalated(irreversible)`, never `claim_accepted`. Human `decision accept` → `claim_accepted` |
| CL-13 | Irreversible claim reaches the drop line | `claim_dropped` with no human |
| CL-14 | B depends on A; A dropped | B `claim_voided(by: A)` in the same transaction; evidence on B, even executed, is recorded and B stays `void` |
| CL-15 | C → B → A chain; A dropped | B and C both voided, atomically (fixed point) |
| CL-16 | B (depends on A) reaches the accept line while A is open; then A accepted | B stays `open`, then `claim_accepted` when A is accepted |
| CL-17 | B accepted and acted on (mark `done`), then its dep A is reopened by the human and dropped | B `claim_escalated(dep_collapsed)`, not voided or reopened |
| CL-18 | Six alternating ±1 opinions on a claim (net movement ≤ 2) | `claim_escalated(stuck)` on the sixth |
| CL-19 | `a` and `b` alternate opinion support strength 1 on a low-stakes reversible claim (score climbs, never accepts: no non-opinion support) | 8th item → `claim_escalated(budget)`; 9th → `claim_budget`; `budget_extended(claim)` + `reopen` → accepts more |
| CL-20 | Budget arithmetic: high stakes and irreversible | Budget = 64 |
| CL-21 | Question Q with a single candidate X reaches the accept line; separately, with a second candidate Y present | `claim_accepted` with `sole_candidate: true`; with Y present, `claim_accepted` without it. No escalation either way |
| CL-22 | Two evidence posts race; the second append hits a stale revision. (a) The first does not settle the claim. (b) The first settles it | (a) Driver reloads; both land; the score is the sum in commit order. (b) The second (an opinion) is recorded on recompute; the settled claim's status and score are unchanged (extends TR-29) |
| CL-23 | Replay with the same `op` after the claim settled | Original result, no new event, no charge (R15) |
| CL-24 | Property: random evidence sequences over random DAGs | Fold equals replay. `void` is left only by human `reopen`; `dropped` and `accepted` are left only by human `reopen`, executed evidence against the outcome, or (for `accepted`) a dep leaving `accepted`. A superseded claim never reopens by evidence. Recorded evidence never changes a score. `spent(c) ≤ budget(c)` always. A run debit alone never changes the status of a settled claim (on an `open` claim it may escalate `budget`, as in CL-42(a)). Voids are closed under deps. No float anywhere in the fold |
| CL-25 | Property: random committed logs, each replayed twice; plus the a,b,c vs a,c,b example | Identical statuses and scores on every replay. The example: a,b,c accepts at b, then c reopens it into epoch 2 at −8; a,c,b stays `open` at 0 in epoch 1 |
| CL-26 | `claims_delta(after_rev, ["http"])` | Returns changed claims in `http` and their dependents, nothing else |
| CL-27 | `claim_line` of an accepted claim | Matches the golden one-line format |
| CL-35 | A `scoped` role calls `claims_delta`, `claim_card` and `claim_line` on another agent's claim, fetches it by id, and calls `claim_card` on a claim the human addressed to it that depends on, and has evidence and a candidate from, another agent | Hidden claim: nothing returned, nothing counted. Addressed claim: returned with the other agent's evidence, dependency line and candidate omitted. No credential → `bad_credential`. The human-channel view returns everything |
| CL-36 | An `all` reader makes the same calls | Full card, lines and delta, including nested evidence, dependencies and candidates its `read_artifacts` allows |
| CL-28 | Message budget crossed by a turnless evidence post | Exactly one task `escalated(messages)`; crossing write accepted; the next write by anyone → `wrong_state` until `budget_extended` + `resumed(open)` (TR-16 in claims mode) |
| CL-29 | End to end with normal APIs: open a claims task; `a` posts claim X; `a` stores two diffs and a candidate naming X; the runner stores a `passed` log and report for the started run; `a` and `b` post `executed` support citing that report | `run_started` on the candidate; report accepted; X `claim_accepted`. No turn is ever granted |
| CL-30 | Same path with `b`'s credential on `a`'s `put_artifact`, an expired credential after reassignment, and a candidate stored in a stage-mode task with a `claim` field | `bad_credential`, `bad_credential`, `wrong_mode`; nothing stored |
| CL-31 | The runner stores a report with a wrong secret, for a revoked run after `rerun`, or a second final report | `bad_run_secret`, `stale_run`, `duplicate_report` |
| CL-32 | `executed` evidence on claim Y citing a report of a candidate naming X; `support` citing a `failed` report; evidence citing an `error` report | `evidence_unbound`, `not_passed`, `evidence_unbound` |
| CL-33 | A new claim names a `dropped` dep, then a `void` dep | `dep_closed` both times. Naming an `escalated` dep is accepted |
| CL-34 | Stage-mode regression: the full TR-01..TR-31 catalogue on a build with claims mode compiled in | All pass unchanged; stage-mode `put_artifact` without a turn is still `bad_token` |
| CL-37 | Accepted claim X receives cited counter, opinion counter, and executed support | All three recorded; status, score and confidence unchanged; no event |
| CL-38 | Accepted claim X, not acted on; `c` stores a re-check candidate naming X; its run reports `failed`; `c` posts executed counter citing it | `run_started`, one unit debited; `claim_reopened(by: evidence)`, epoch 2, score −8. One further support from `a` does not re-accept (one author in epoch 2) |
| CL-39 | Same as CL-38, but the host recorded `claim_acting(X, 1, pr)` first | `claim_escalated(reopened_after_action)`, not `claim_reopened`. Human `reopen` → `open`, new epoch |
| CL-40 | Dropped claim receives executed support citing a `passed` report | `claim_reopened(by: evidence)`, new epoch, score +4 |
| CL-41 | B (accepted, not acted on) depends on A (accepted); A reopened by executed counter | A `open`; B `claim_reopened(by: A)`, new epoch, `open`, all in one transaction |
| CL-47 | B and C accepted, both depend on A; C acted on (mark `done`); A dropped by executed counter evidence (via reopen, then drop line) | B reopened then voided in the same transaction; C `claim_escalated(dep_collapsed)` |
| CL-48 | D depends on B, B depends on A, all accepted, none acted on; A reopened | B and D both reopened, atomically (fixed point) |
| CL-49 | Agent posts Y with `supersedes: X` right after X is accepted (X acted on, irreversible); Y stays open; then a failed report on X | X still `accepted`, so the failed report escalates `reopened_after_action` as usual. Later, Y reaches its accept line → `claim_escalated(supersedes_acted)`; human `accept` → Y accepted, X `superseded` with its marks; further counter evidence on X is recorded only. A second `supersedes` naming X → `bad_claim` |
| CL-53 | Open claim X with an open dependent D and a held rerun; Y supersedes X; Y accepted | Before Y's acceptance nothing changes; on it, X `claim_superseded(by: Y, prior: open)`, the held rerun withdrawn, D voided, one transaction. If Y is dropped instead, X is untouched |
| CL-54 | High-stakes claim: two passing tests (8), then a third (12); an irreversible low-stakes claim the same | Open at 8, accepted at 12; the irreversible one escalates `irreversible` at 12, not at 8 |
| CL-56 | B accepted on its last budget unit, not acted on, depends on A; A reopened by executed counter | B `claim_reopened(by: A)` and `claim_escalated(budget)` in the same transaction; also through a chain C → B → A with C and B both at their limit (both escalate). Replay of the reopening `op` returns the original result with no second escalation; B's budget is never reset or extended without a human `budget_extended` |
| CL-57 | X superseded by Y; then an agent posts B with `deps: [X]`; separately, Z posted with `supersedes: X` and `deps: [W]` where W depends on X | `dep_closed`; `bad_claim` (Z rests on what it retires, transitively) |
| CL-58 | Y1 and Y2 both posted with `supersedes: X` while X is open; Y1 accepted, then Y2 accepted; separately, X becomes acted on between Y3's posting and its accept line | X `claim_superseded(by: Y1)`; Y2 `claim_accepted` with `supersedes_lapsed: X`, X unchanged. Y3's accept line escalates `supersedes_acted` (marks checked at the commit) |
| CL-55 | Question Q with candidates A (open), B (escalated `stuck`), C (accepted, acted on), D (void); human `choose(Q, B)` | B accepted; A and C dropped, C keeps its mark; D unchanged; dependents of A and C follow Dependencies; one transaction; a stale `rev` → `stale_rev`, nothing written |
| CL-50 | Dropped claim X (not superseded) receives executed support citing a passed report | `claim_reopened(by: evidence)` (symmetry; same as CL-40) |
| CL-51 | Mark on X with no result. (a) `mark_timeout_secs` passes. (b) The human then records `aborted`. (c) A result arrives before the timeout | (a) `claim_escalated(action_unconfirmed, mark)`; X keeps its status. (b) The mark is cleared; X is no longer acted on. (c) No escalation |
| CL-52 | A `source` artifact stored turnless, then `cited` evidence citing it; the same `source` in a stage task; a `source` with a bad digest | Accepted with weight 2; `wrong_mode`; refused, nothing stored |
| CL-42 | Candidate debits. (a) Open low-stakes reversible claim with 7 evidence items; a candidate naming it is stored. (b) A claim accepted on its 8th unit; a re-check candidate naming it. (c) A candidate naming a `void` or `escalated` claim. (d) An irreversible claim accepted by a human `decision` (after CL-12), with budget left; a re-check candidate naming it | (a) `run_started`, `spent` = 8, `claim_escalated(budget)` in the same transaction. (b) `claim_budget`, no run. (c) `claim_not_open`, no run. (d) `run_started`, one unit debited, the claim stays `accepted`, no `claim_escalated` |
| CL-43 | `claim_acting` on an `open` claim, or naming an old epoch; `claim_action_result` for an unknown mark; a second result for one mark | `wrong_state`; `ref_unresolved`; `duplicate` |
| CL-45 | X accepted in epoch 1. (a) Host marks `claim_acting(X, 1, pr)`; contrary executed evidence commits; host records `done`. (b) Contrary evidence reopens X first; host then tries to mark epoch 1. (c) As (a), then the human reopens, X is accepted in epoch 2, and the epoch-1 result arrives late | (a) `claim_escalated(reopened_after_action)`; the result lands on the epoch-1 mark. (b) `wrong_state`, no mark, so the host does not act. (c) The result binds to the epoch-1 mark, not epoch 2; X stays acted on |
| CL-46 | Run budget on reruns: candidate K on claim X. (a) One unit left: human `rerun(K)`. (b) Exactly at the limit: human `rerun(K)`. (c) Replay of (a)'s `op`. (d) Approve the held rerun from (b). (e) At the limit, an agent stores a new candidate on X | (a) Old run revoked, new `run_started`, one unit debited. (b) `rerun_held(X, K)`, X's status unchanged, the old run's report is still accepted. (c) Original result, no second debit. (d) `budget_extended(X)` adds one `base(X)`, the old run is revoked, a new run starts, one unit debited. (e) `claim_budget`, nothing written |
| CL-59 | Low-stakes claim X, candidate K with profiles `unit`, `lint`. (a) K's run fails `unit`; `a` and `b` post support from two earlier passing runs of K, score 8. (b) Rerun of K passes listing only `lint`. (c) Rerun of K passes listing `unit` and `lint`. (d) Accepted X re-checked: the re-check run fails `unit`, `c` posts executed counter | (a) X stays `open`, `failing(X) = {unit}`. (b) Still `open`, `failing(X) = {unit}` (a reduced suite does not clear). (c) `failing(X)` empty; X `claim_accepted` on that report's commit with no new evidence. (d) `claim_reopened(by: evidence)`, and X is blocked in epoch 2 (`failing(X) = {unit}`) |
| CL-60 | Claim X, candidate K1 fails `unit` (`failing(X) = {unit}`); K2 stored naming X. (a) Support reaches 8 before K2 reports. (b) K2's run passes listing only `lint`. (c) K2's run passes listing `unit`. (d) Then K1's rerun fails `unit` and `lint` | (a) X stays `open`: K2 is current and inherits the block. (b) Still blocked. (c) `failing(X)` empty, X `claim_accepted`. (d) K1 is not current, `failing(X)` stays empty, X stays `accepted` |
| CL-61 | Claim X blocked with `failing(X) = {unit}`. (a) The runner stores a `passed` report with a `profile_digest` other than the task's. (b) Host test: the frozen profile file is edited to drop `unit`, then a rerun starts. (c) An agent tries `put_artifact` of any kind that would change profiles | (a) `profile_mismatch`, nothing stored, `failing(X)` unchanged. (b) The runner refuses the set (digest mismatch) and no report is stored. (c) No Agent API operation writes profiles: `kind_forbidden` |
| CL-62 | Claim X, `failing(X) = {unit}`, `test_paths = ["tests/**"]`. (a) Candidate K2 by `a` modifies `tests/core.rs`; its run passes listing `unit`. (b) K3 deletes `tests/core.rs`, passes. (c) K4 only adds `tests/new.rs`, passes listing `unit`. (d) Adversarial: K5 changes no existing test and adds `tests/conftest.py` whose autouse fixture skips the failing test, so `unit` exits 0 with no failures. (e) K6 changes no file under `test_paths`, passes listing `unit` | (a) Report `tests_changed: [tests/core.rs]`; `failing(X)` stays `{unit}`. (b) Same, `tests_changed: [tests/core.rs]`. (c) `tests_changed: [tests/new.rs]`; `failing(X)` stays `{unit}` until endorsed as in CL-63, then cleared. (d) `tests_changed: [tests/conftest.py]`; `failing(X)` stays `{unit}`, and executed support citing the report is `tests_unendorsed`. (e) `tests_changed` empty; `failing(X)` cleared. (f) On an unblocked claim Y, a candidate that only adds `tests/new.rs` passes: `tests_changed: [tests/new.rs]`, and executed support citing it is admitted without endorsement |
| CL-63 | K2 as CL-62(a). (a) `b` posts executed support citing K2's report. (b) `a` (K2's author) posts cited support citing K2, then `b` retries. (c) `c` (author of neither K2 nor its diffs) posts cited support citing K2, then `b` retries. (d) Variant: K3 stored naming X before `c`'s endorsement of K2 | (a) `tests_unendorsed`. (b) `a`'s post is admitted as cited support but is no endorsement; the retry is still `tests_unendorsed`. (c) Endorsement recorded; `failing(X)` cleared at the endorsement's commit (K2 still current, its passing report still latest); `b`'s executed support admitted and scored, and X is accepted once score and authors reach the line. (d) As (c), but K3 was stored before `c` endorses K2: no clearing, since K2 is no longer current |
| CL-71 | Holdout task, roster adds holdout lens `h`. Claim X with current K. (a) `a` stores a holdout candidate of K. (b) `h` stores one with another base. (c) `h` stores one with K's manifest. (d) On a task without a holdout suite, `h` stores one | (a) `kind_forbidden` naming `write:holdout`. (b) `holdout_mismatch`. (c) Stored, one unit debited, a run started, `current(X)` still K. (d) `holdout_mismatch`; two build passes accept as before |
| CL-72 | Holdout task. X with K passing; `a` and `b` post executed support (8). Then `h`'s holdout run of K passes. Then claim Y with cited support only | X stays `open`, card `holdout_due`; accepted at the holdout report's commit. Y (no candidate) accepts without a holdout |
| CL-73 | Holdout task. K's holdout run fails `holdout:api`; K2 stored and passes `unit`; support reaches 8; then K2's holdout run passes listing `holdout:api` | `failing(X) = {holdout:api}`, set by the holdout report; K2 inherits it and its build pass cannot clear it; X `open` until the holdout pass, then `accepted` |
| CL-74 | Holdout task. Build runs report under `holdout_digest`, or list `holdout:api` or `golden:cases`; holdout runs report under `profile_digest`, or list `unit` | `profile_mismatch` for a wrong digest, `inconsistent_report` for a profile of the other suite; a holdout report listing `golden:cases` is stored |
| CL-75 | Holdout task. `h` stores a holdout candidate of K1, then K2 becomes current; the holdout run fails; support reaches 8 on K2 | No block (replaced code), no holdout pass either: X stays `open`; a human `accept` overrides |
| CL-77 | Holdout task. X accepted on v1; Y passed v1's holdout but lacks support; Z blocked by a v1 holdout failure. (a) Host `holdout_extended(v2)`, then support reaches 8 on Y, then a holdout report under v1, then one under v2. (b) `holdout_extended(v2)` again; human `holdout_updated(v3)`. (c) Either on a task without a holdout suite | (a) X stays `accepted`; `failing(Z)` loses its holdout entry; Y stays `open` with `holdout_due`; the v1 report is `profile_mismatch`; the v2 pass accepts Y. (b) `duplicate`; v3 adopted, log replays. (c) `holdout_mismatch` |
| CL-78 | Claim X in domain `tests`; `b` writes the diff, `a` the candidate; executed support from `a` and `b` reaches 8; then `c` posts support | X stays `open` until `c` (author of neither) supports it, then `accepted`. A claim outside `tests` accepts on its authors' support as before |
| CL-79 | Host test, `protected = ["tests/**"]`. (a) A candidate whose diff touches `flag.txt` and adds, modifies and deletes files under `tests/`. (b) One touching only `flag.txt`. (c) On a `tests` claim T: a mixed candidate, then a test-only one. (d) Another claim lists T's diff before and after T is accepted. (e) A fresh diff to the same test path | (a) `protected_path` naming the three test paths, not `flag.txt`; no run, no current candidate. (b) Stored. (c) Mixed refused, test-only stored. (d) Refused, then stored. (e) Refused. Also: an open with `protected` but no `baseline`, or with `tests/[ab].rs` or `/tests/**`, is `bad_claims_config(protected)`; a candidate on another base is `protected_path`; after T is dropped the committed reuse op replays `stored`, a fresh one is refused |
| CL-94 | A driver holding a stale revision dispatches with a guard that refuses only on the newer revision; then with a guard that always passes | The append conflicts, the reloaded state fails the guard: refused, nothing appended. The second dispatch commits |
| CL-93 | Holdout task. Y has a candidate and passed v1's holdout; claim B has none. Host `holdout_extended(v2)`; then `holdout_extended(v1)` and human `holdout_updated(v1)` | The delta after the prior rev lists Y, not B. Both returns to v1 are `duplicate`; `holdout_digest` stays v2; the log replays |
| CL-80 | Host test, sandboxed. Holdout task, suite `holdout:api` printing a marker. (a) A builder stores a `suite: holdout` candidate. (b) `h` stores one; the runner serves it. (c) A file is planted in the suite and `h` stores another. (d) `bbp holdout` with an added file, then with a changed file, then the human `holdout-updated` of the named digest | (a) `kind_forbidden`. (b) Report under `holdout_digest`; the log artifact is the summary without the marker; the host-only log has it. (c) That run reports `error`. (d) `holdout_extended` written; then `human_required` naming the staged digest, digest unchanged; then adopted |
| CL-90 | Host test, sandboxed. (a) `bbp open --golden` with an unsigned set. (b) A signed set whose case cats `flag.txt` expecting `ok`; a candidate makes the flag `ok` and `h` stores its holdout candidate | (a) `bad_golden`, nothing written. (b) The suite's profile is `golden:flag`; the holdout run passes and its host-only log says `pass says-ok` |
| CL-76 | (a) A stage task opens with `holdout_digest`. (b) A holdout task's log replays | (a) `bad_claims_config`. (b) The fold reproduces the claims book, `holdout_digest` included |
| CL-91 | Holdout task. K's build report tested tree T1; support reaches 8; `h`'s holdout run of K passes on tree T2, then another on T1 | X stays `open` and `holdout_due` after the T2 pass; accepted at the T1 pass |
| CL-92 | Holdout task. K's holdout run fails `holdout:api`; K2 stored; its holdout run passes listing `holdout:api` with a modified test; `c` endorses K2 before (or after) that report | Endorsed first: the report clears `holdout:api`. Endorsed after: the block stays until the endorsement, which clears it |
| CL-44 | Host test (`rusty_bbp_host`): open a claims-mode task on a host with no sandbox configured; on a sandboxed host, inspect where a candidate's run executes | `bad_claims_config(sandbox)`, nothing written; the run's process is inside the `rusty_sandbox` profile |

## Owner answers (2026-11-02)

| Id | Question | Answer |
| --- | --- | --- |
| Q1 | How is an `executed` run requested in claims mode? | **Accepted.** Storing a candidate that names a claim starts its run, debited one unit from that claim |
| Q2 | Track-record weighting? | **Accepted.** None in v0: every author weighs 1, and only distinct principals are counted |
| Q3 | Accepted claim whose dep collapses? | **Changed by the owner.** It reopens automatically if nothing acted on it; only an acted-on claim escalates `dep_collapsed` (Dependencies) |
| Q4 | Default numbers? | **Accepted with two changes:** high-stakes and irreversible claims accept at 12; an executed failing test counts −8. Everything else as restated below |
| Q5 | `sole_candidate` in BBP or orch? | **Accepted, as a fact.** BBP records it on `claim_accepted`; orch decides what to do about it |

**Q4 in plain terms (accepted 2026-11-02):**
- `accept_at 8` means **90%** shown confidence; `accept_at_strict 12` (high-stakes or irreversible) means **96%**; `drop_at -8` means **10%**. p = 1/(1+3^(−score/4)).
- **Two passing tests from two different authors reach accept; three for high-stakes or irreversible claims.** A passing test adds 4 and a failing test subtracts 8. One author alone never accepts or drops (`min_authors 2`). Cited evidence (weight 2) needs twice as much: for example two strength-2 citations from two authors. Opinions alone never accept.
- `settle_window 6` and `settle_delta 2` count **evidence items, not rounds**: if the last 6 scoring items in the current epoch moved the score by 2 or less in total, the claim escalates `stuck`. Recorded (non-scoring) items and runs do not count.
- `evidence_base 8`: **one unit buys one evidence post or one test run.** A claim gets 8, 16 or 32 units for low, medium or high stakes, doubled if irreversible.
- `mark_timeout_secs 3600`: an action mark with no result for an hour escalates.

# BBP claims addendum (draft, v0)

Nov 2, 2026 · **Status: approved by the owner on 2026-11-02, including the schema and API changes listed below. Second owner review (2026-11-02): order-dependent settlement accepted; settled claims keep taking evidence and executable evidence reopens them; candidate runs approved if debited from the claim budget and sandboxed. Open questions Q1 to Q5 stand at their proposals, listed with their answers at the end. Spec only; the engine change is a separate PR.**
Extends [`bbp-v0.5-implementation-spec.md`](bbp-v0.5-implementation-spec.md). Card: Mill Board #593 (Let Em Loose).

## Summary

A task may open in **claims mode**. Its agents post **claims** (small propositions) and **evidence** for or against them; the board, not the agents, computes each claim's score and settles it: accepted, dropped, void (a dependency fell) or escalated to the human. Evidence and claims are append-only and need **no turn**, so any number of agents post concurrently. Everything that changes state (a claim settling, an escalation, a human decision, a budget extension) stays serialized through the single transactional writer, as today.

Claims mode sits **beside** the stage lifecycle. A claims-mode task has no `planning … merge_gate` states. A task opened without `mode` is a stage task, byte for byte as before.

Owner decisions this rests on (2026-11-02): evidence posts need no turn; the one-live-turn limit stays for state changes, budgets and approvals; claims sit beside the stage lifecycle in v0, chosen per task; existing tasks are unchanged. Roles, lenses, model choice and track records stay in orch.

## Where this starts from (main b8467139)

| Today | Claims mode |
| --- | --- |
| Task lifecycle = the factory pipeline (`planning` → `plan_gate` → `build` → `test` → `review` → `merge_gate`) | Task states `open`, `escalated`, `closed`, `cancelled`. Each claim has its own status |
| Every agent write needs a live turn token (R14) | `claim`, `evidence`, `read`, `get_artifact` and `put_artifact` of `diff`/`candidate` are turnless, authorised by a per-assignment session credential (C-R1) |
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
    "accept_at": 8, "drop_at": -8,
    "min_authors": 2,
    "settle_window": 6, "settle_delta": 2,
    "evidence_base": 8
  }
}
```

- `mode` absent means `stages`: no field is written, the schema stays 2, 3 or 4, and the log is byte-identical to today.
- `mode: "claims"` writes **schema 5**. A reader older than this addendum refuses schema 5 rather than replaying it as a stage task.
- `claims` is validated at open: `drop_at < 0 < accept_at`, `min_authors ≥ 1`, `settle_window ≥ 2`, `settle_delta ≥ 0`, `evidence_base ≥ 1`. A bad value refuses the open as `bad_claims_config`, naming the field. The values above are the defaults; they are judgment, not data, and are fixed before the first run.
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
| `answers` | Optional earlier `claim` this one is a candidate answer to (an open question). Candidates of one question compete (C-R7) |
| `stakes` | `low`, `medium` or `high` |
| `reversible` | `false` means the board never accepts it alone (C-R6) |
| `refs` | At least one (R2), usually the brief or an artifact |

A claim starts `open` with score 0 (neutral).

### `evidence`

```json
{
  "kind": "evidence", "to": ["*"], "refs": ["msg:17", "art:01HDIFF#L40-L52"],
  "evidence": ["art:01HREP"],
  "body": { "claim": "msg:17", "direction": "support", "kind": "executed", "strength": 2, "note": "…" }
}
```

| Field | Rule |
| --- | --- |
| `claim` | An earlier `claim` on the task, and it must appear in `refs` |
| `direction` | `support` or `counter` |
| `kind` | `executed`: `evidence` cites a runner `test_report` (written by the runner principal, so an agent cannot fake one). `cited`: `evidence` cites at least one artifact or message range. `opinion`: no `evidence` required |
| `strength` | 1, 2 or 3, the author's own rating |
| `note` | 0 to 1,200 characters |

Evidence on a settled claim is still admitted (see Evidence after settlement). There is no retraction: a correction is new evidence in the other direction.

## The update rule (deterministic, integer only)

The score of a claim is a fold over its evidence in log order. There are no floats and no clock, so replay reproduces it exactly.

A claim's evidence is split into **epochs**. The first epoch starts when the claim is posted; each reopen (by evidence or by a human) starts a new one. The update rule reads only the current epoch. The card shows every epoch.

```
weight(kind)  = executed 4 | cited 2 | opinion 1
delta(e)      = sign(direction) × weight(kind) × strength        # ±1 … ±12
score(c)      = Σ delta(e) over scoring evidence in c's current epoch
authors(c, +) = distinct principals with scoring support in the current epoch
authors(c, −) = distinct principals with scoring counter in the current epoch
```

Evidence is **scoring** when it is admitted while the claim is `open`, or when it reopens the claim (it is then the first item of the new epoch). Evidence recorded on a settled claim without reopening it is **recorded**: stored, shown, charged, never scored.

**Confidence** is shown as `p = 1 / (1 + 3^(−score/4))`, rounded to whole percent. It is for people only; no rule reads it. With the defaults, `accept_at = 8` is about 90% and `drop_at = -8` about 10%.

After each scoring `evidence`, and after each candidate debit (Budget), the board evaluates that claim in this order. The first rule that fires wins, and its event is generated in the same transaction (Processing model, phase 4):

1. **Accept line.** `score ≥ accept_at`, `authors(c,+) ≥ min_authors`, at least one support item is not `opinion`, and every dep is `accepted`:
   - if `reversible: false`: `claim_escalated(reason: irreversible)` (C-R6);
   - if it is a candidate whose question has fewer than two candidates: `claim_escalated(reason: sole_candidate)` (C-R7);
   - otherwise `claim_accepted`.
2. **Drop line.** `score ≤ drop_at`, `authors(c,−) ≥ min_authors`, and at least one counter item is not `opinion`: `claim_dropped`. Dropping an irreversible claim needs no human, because not doing something can be undone.
3. **Budget.** `spent(c)` reaches `budget(c)`: `claim_escalated(reason: budget)`.
4. **Settling.** The current epoch has at least `settle_window` scoring items, and `|score now − score settle_window items ago| ≤ settle_delta`: `claim_escalated(reason: stuck)`.

A claim that crosses the accept line while a dep is still `open` stays `open`. It is re-evaluated whenever a dep settles (C-R5).

### Settlement follows committed order

The rule is evaluated per commit, in log order, against the claim as it stands at that commit. A settlement takes effect at the commit that causes it. Evidence committed after it is judged against the settled claim (Evidence after settlement), even if its author sent it before the settling post committed.

**The guarantee is that the same committed log always replays to the same statuses, epochs and scores.** Order independence is not guaranteed (owner decision, 2026-11-02): the same evidence committed in a different order can settle differently. With the defaults, executed +4 from `a`, +4 from `b` and executed −4 from `c` (each citing its own run):
- in the order a, b, c, the claim is accepted at b; c then reopens it, and it is `open` in a new epoch at −4;
- in the order a, c, b, it is never settled and stays `open` at +4.

That is intended: the board decides on what it had when it decided. Nothing is lost silently. Every attempted post is either committed or refused with a code the author sees, and a refusal is logged as `rejected` like any other.

### Evidence after settlement

A settled claim keeps taking evidence. What that evidence does depends on its kind and on the settled status:

| Claim status | Executed evidence against the outcome | Any other evidence |
| --- | --- | --- |
| `accepted` | `counter` citing a `failed` report **reopens** it | Recorded |
| `dropped` | `support` citing a `passed` report **reopens** it | Recorded |
| `escalated`, `void` | Recorded (the human decides an escalation; a void claim waits for its dep) | Recorded |

- "Executed" means a runner `test_report` bound to the claim (C-R4). Tests, holdout sets and golden sets are all runner profiles and all produce one. Cited and opinion evidence never reopen a claim and never move its confidence.
- **Reopen by evidence.** The board writes `claim_reopened(by: evidence)` in the same transaction. The claim is `open`, a new epoch starts, and that evidence is its first scoring item (so a counter reopen starts below zero). It must earn its way back to a settlement with fresh evidence from `min_authors` principals in the new epoch.
- **Reopening a claim that was acted on escalates instead.** If the claim carries a `claim_acted` event (its acceptance was used, for example merged), the same evidence writes `claim_escalated(reason: reopened_after_action)` and the claim goes to the inbox. The human then decides `reopen`, `accept` or `drop` (C-R8).
- **Dependents.** When an accepted claim leaves `accepted` (reopened, or escalated after action), each accepted dependent is escalated with `dep_collapsed`, as for a dropped dep. Open dependents stay open and stop meeting the accept line until the dep is accepted again.
- Every evidence item, recorded or scoring, counts toward `spent(c)`. At `spent(c) ≥ budget(c)` further evidence on the claim is refused `claim_budget` until the human records `budget_extended(claim)`.

**No rule reads who the author is beyond counting distinct principals.** Track-record weighting is out of v0 (open question Q2).

### Budget

```
budget(c) = evidence_base × stakes_factor × reversibility_factor + Σ budget_extended(c)
stakes_factor        = low 1 | medium 2 | high 4
reversibility_factor = reversible 1 | irreversible 2
spent(c)  = evidence items on c (all epochs) + runs started for candidates naming c
```

The defaults give 8 to 64 units per claim. One evidence item costs 1 and one run costs 1, so a run is debited from the claim it tests (owner decision, 2026-11-02). A candidate that would start a run on a claim with `spent(c) ≥ budget(c)` is refused `claim_budget` and no run starts. Task totals (`messages`, `bytes`, `reads`) still apply and are charged as today (R11), since `claim` and `evidence` are messages. The `turns` and `iterations` budgets are not used in claims mode.

### Dependencies

- When a claim becomes `dropped` or `void`, every claim that lists it in `deps` and is not yet settled becomes `void` (`claim_voided(by)`). This repeats to a fixed point in the same transaction, so a collapse is atomic.
- An already-`accepted` dependent is **not** reopened automatically. It is escalated with `reason: dep_collapsed`, because acceptance may already have been acted on. The same applies when a dep leaves `accepted` by a reopen (Evidence after settlement). (Open question Q3.)
- When a dep becomes `accepted`, each `open` dependent is re-evaluated against the accept line.

## Rules (claims mode)

| Id | Rule | Rejects with |
| --- | --- | --- |
| C-R1 | In claims mode, `post` of `claim` or `evidence`, `read`, `get_artifact`, and `put_artifact` of `diff` or `candidate` need no turn. They carry the **session credential** issued to the principal at `assigned` and invalidated when the role is reassigned or the task ends. Credential binding replaces R14 for exactly these operations in claims mode. Any other Agent API mutation in claims mode is refused, and stage mode keeps R14 unchanged. | `bad_credential`, `wrong_mode` |
| C-R2 | The task is in `open` (not `escalated` on a task total, not terminal). A stage-mode task refuses both kinds. | `wrong_state`, `wrong_mode` |
| C-R3 | Body fields as in Records; `deps`, `answers` and `claim` resolve to `claim` messages on the task (R4). A new claim may not name a `dropped` or `void` dep, since it would be born dead; an `open`, `accepted` or `escalated` dep is fine. | `bad_claim`, `ref_unresolved`, `wrong_kind`, `dep_closed` |
| C-R4 | Evidence targets any claim on the task with `spent(c) < budget(c)`; whether it scores, reopens or is recorded follows Evidence after settlement. `executed` cites a final runner `test_report` of a run whose candidate names the same claim; `support` needs that report `passed`, `counter` needs it `failed` (an `error` report is not evidence). `cited` cites at least one artifact or message. | `claim_budget`, `evidence_required`, `evidence_unbound`, `not_passed` |
| C-R5 | Settling is generated by the board (update rule), never posted by an agent. | n/a |
| C-R6 | An irreversible claim is never accepted without a human `decision`. | n/a (escalates) |
| C-R7 | A candidate claim (`answers` set) is accepted only while its question has at least two candidates. | n/a (escalates) |
| C-R8 | A human `decision` with `subject` a claim and `outcome` `accept`, `drop` or `reopen` settles or reopens it (a reopen starts a new epoch). `reopen` on a budget escalation needs a `budget_extended(claim)` first. Human decisions bypass `min_authors` and thresholds; they are the override. | `stale_rev`, `wrong_state` |
| C-R9 | `claim_acted(claim, ref)` is written by the host or the human, only on an `accepted` claim, at most once per epoch. `ref` names what used it (a PR, a merge commit). | `wrong_state`, `duplicate` |

R1 to R13, R15 and R18 apply unchanged, except that R11's per-turn read charges become per (principal, artifact id) in claims mode and `reads_per_turn` does not apply. R17 never fires in claims mode (no gates). R16 is replaced in claims mode by the gating rows below; in stage mode it is unchanged.

## Runs and candidates (claims mode)

This is the path from a prototype to `executed` evidence, using the normal APIs.

| Kind | Writer | Credential | Rule |
| --- | --- | --- | --- |
| `diff` | role with `write` ∋ `diff` | session credential | Envelope limit. No state gate (claims mode has no `build`) |
| `candidate` | role with `write` ∋ `candidate` | session credential | Manifest `{base, diffs, claim}`. `claim` is **required** in claims mode and **refused** in stage mode, so stage gating is unchanged. `claim` must be an `open`, `accepted` or `dropped` claim on the task (a candidate on a settled claim is a re-check, for example a holdout run against an accepted claim), with `spent(c) < budget(c)` (`claim_not_open`, `claim_budget`); every `diffs` entry must be a `diff` on the task (R4). Storing it debits one unit from the claim (Budget) |
| `test_report`, `log` | runner | run secret | As R16: names its run and that run's candidate, presents the run's secret, is the first final report or log of that run, and `passed` needs every profile clean with `profile_digest` equal to the task's |
| `spec`, `brief` | no agent | | `spec` is refused in claims mode (`wrong_mode`). `brief` is human-only at open, as now |

- **Each stored candidate gets its own run.** In the same transaction the moderator records `run_started(candidate, run)` and hands the supervisor the run id and secret out of band, as today. Several candidates may have pending runs at once; how many execute together is the host's admission slots, not a protocol limit. A candidate has at most one live run. A human `rerun(candidate)` revokes it and selects a fresh one (T12's effect, no state change), and the old secret is dead, so a late report is `stale_run`.
- **Sandbox only.** A claims-mode run executes only inside the host's sandbox (`rusty_sandbox`, through `rusty_worktree`), never on the host directly. This is a host requirement, since BBP cannot see where a run executes: a host that cannot sandbox refuses to open a claims-mode task (`bad_claims_config(sandbox)`), and the runner principal is the only writer of reports.
- **Reads.** `get_artifact` needs the session credential and the role's `read_artifacts`, as in stage mode.
- **Binding.** An `executed` evidence item binds to one final report, and through it to one run, one candidate and one claim (C-R4). Evidence citing a report from a different claim's candidate, a non-final report or a report the runner did not write is `evidence_unbound`.

## Events (new)

| Event | Writer | Payload |
| --- | --- | --- |
| `claim_accepted` | moderator | claim, score, evidence count |
| `claim_dropped` | moderator | claim, score, evidence count |
| `claim_voided` | moderator | claim, by (the dep that fell) |
| `claim_escalated` | moderator | claim, reason: `irreversible`, `sole_candidate`, `budget`, `stuck`, `dep_collapsed`, `reopened_after_action` |
| `claim_reopened` | human or moderator | claim, by (`decision` or the reopening evidence), new epoch number; a human reopen also carries the expected `rev`. Reopening a claim does not revive dependents it voided; each needs its own `reopen` |
| `claim_acted` | host or human | claim, ref (C-R9) |
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
7. **`claim_acted`** is a new host/human write, and **sandboxed runs** are a host requirement for claims mode (orch-bbp runs candidates through `rusty_sandbox` only).

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
| CL-07 | `a` and `b` each post executed support strength 1 (score 8, 2 authors) | `claim_accepted` in the transaction of the second post |
| CL-08 | `a` alone posts executed support strength 3 twice (score 24, 1 author) | Stays `open` (`min_authors`) |
| CL-09 | `a`, `b` post opinion support until score ≥ 8 | Stays `open` (no non-opinion support) |
| CL-10 | `a`, `b` post executed counter to −8 | `claim_dropped` |
| CL-11 | `executed` evidence citing a `test_report` the runner did not write, or citing a `diff` | `evidence_unbound` / `wrong_kind` |
| CL-12 | Irreversible claim reaches the accept line; another claim keeps taking evidence meanwhile | `claim_escalated(irreversible)`, never `claim_accepted`. Human `decision accept` → `claim_accepted` |
| CL-13 | Irreversible claim reaches the drop line | `claim_dropped` with no human |
| CL-14 | B depends on A; A dropped | B `claim_voided(by: A)` in the same transaction; evidence on B, even executed, is recorded and B stays `void` |
| CL-15 | C → B → A chain; A dropped | B and C both voided, atomically (fixed point) |
| CL-16 | B (depends on A) reaches the accept line while A is open; then A accepted | B stays `open`, then `claim_accepted` when A is accepted |
| CL-17 | B accepted, then its dep A is reopened by the human and dropped | B `claim_escalated(dep_collapsed)`, not voided |
| CL-18 | Six alternating ±1 opinions on a claim (net movement ≤ 2) | `claim_escalated(stuck)` on the sixth |
| CL-19 | `a` and `b` alternate opinion support strength 1 on a low-stakes reversible claim (score climbs, never accepts: no non-opinion support) | 8th item → `claim_escalated(budget)`; 9th → `claim_budget`; `budget_extended(claim)` + `reopen` → accepts more |
| CL-20 | Budget arithmetic: high stakes and irreversible | Budget = 64 |
| CL-21 | Question Q with a single candidate X reaches the accept line | `claim_escalated(sole_candidate)`. With a second candidate Y present, X is accepted |
| CL-22 | Two evidence posts race; the second append hits a stale revision. (a) The first does not settle the claim. (b) The first settles it | (a) Driver reloads; both land; the score is the sum in commit order. (b) The second (an opinion) is recorded on recompute; the settled claim's status and score are unchanged (extends TR-29) |
| CL-23 | Replay with the same `op` after the claim settled | Original result, no new event, no charge (R15) |
| CL-24 | Property: random evidence sequences over random DAGs | Fold equals replay. `void` is left only by human `reopen`; `dropped` and `accepted` are left only by human `reopen`, executed evidence against the outcome, or (for `accepted`) `claim_escalated(dep_collapsed)`. Recorded evidence never changes a score. `spent(c) ≤ budget(c)` always. Voids are closed under deps. No float anywhere in the fold |
| CL-25 | Property: random committed logs, each replayed twice; plus the a,b,c vs a,c,b example | Identical statuses and scores on every replay. The example: a,b,c accepts at b, then c reopens it into epoch 2 at −4; a,c,b stays `open` at +4 in epoch 1 |
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
| CL-38 | Accepted claim X, not acted on; `c` stores a re-check candidate naming X; its run reports `failed`; `c` posts executed counter citing it | `run_started`, one unit debited; `claim_reopened(by: evidence)`, epoch 2, score −4. One further support from `a` does not re-accept (one author in epoch 2) |
| CL-39 | Same as CL-38, but the host recorded `claim_acted(X, pr)` first | `claim_escalated(reopened_after_action)`, not `claim_reopened`. Human `reopen` → `open`, new epoch |
| CL-40 | Dropped claim receives executed support citing a `passed` report | `claim_reopened(by: evidence)`, new epoch, score +4 |
| CL-41 | B (accepted) depends on A (accepted); A reopened by executed counter | A `open`; B `claim_escalated(dep_collapsed)` in the same transaction |
| CL-42 | Candidate debits. (a) Open low-stakes reversible claim with 7 evidence items; a candidate naming it is stored. (b) A claim accepted on its 8th unit; a re-check candidate naming it. (c) A candidate naming a `void` or `escalated` claim | (a) `run_started`, `spent` = 8, `claim_escalated(budget)` in the same transaction. (b) `claim_budget`, no run. (c) `claim_not_open`, no run |
| CL-43 | `claim_acted` on an `open` claim; twice on one accepted claim | `wrong_state`; `duplicate` |
| CL-44 | Host test (`rusty_bbp_host`): open a claims-mode task on a host with no sandbox configured; on a sandboxed host, inspect where a candidate's run executes | `bad_claims_config(sandbox)`, nothing written; the run's process is inside the `rusty_sandbox` profile |

## Open questions: answers in force

The owner's "Approved" was taken to cover these at their proposals. They are listed so the owner can change any of them before the engine change.

| Id | Question | Answer in force |
| --- | --- | --- |
| Q1 | How is an `executed` run requested in claims mode? | Storing a candidate that names a claim starts its run, debited one unit from that claim (Runs and candidates, Budget) |
| Q2 | Track-record weighting? | None in v0: every author weighs 1, and only distinct principals are counted. Later, if wanted: per-principal weights fixed in `task_opened` (immutable, so replay holds), supplied by orch from the lens's record |
| Q3 | Accepted claim whose dep collapses or reopens? | Escalate with `dep_collapsed`, never auto-void, because the acceptance may already have been acted on |
| Q4 | Default numbers? | `accept_at 8`, `drop_at -8`, `min_authors 2`, `settle_window 6`, `settle_delta 2`, `evidence_base 8` (so a budget of 8 to 64 units) |
| Q5 | `sole_candidate` in BBP or orch? | In BBP (C-R7), since it is the one groupthink rule that needs board data. Blind first pass, rebel candidate and look-harder stay in the lens runner (#594) |

# ADR-0001: The BBP conductor: a board client that hosts each card as a Blackboard Protocol task

- **Status:** Paused (2026-10-13): BBP is being revised; revisit when it settles. The generic parts (asks, handles, role-bound credentials) are still useful. A BBP-free runner is built first, see ADR-0002.
- **Date:** 2026-10-13

## Context
Mill Board holds work items and enforces who may move them. The Blackboard Protocol (BBP) already
runs the work: `bbp mod` launches the vendor CLIs per role, `bbp runner` tests candidates in a
sandbox, and `bbp human` is the human channel at the gates. What is missing is the bridge: nothing
turns a card in Ready into a BBP task, keeps the card's column true to the task's state, or lets a
person answer a gate from the board. Calling that bridge "the runner" is misleading, since BBP's
moderator and runner exist; this ADR names it the **conductor**.

Constraints that shape it:
- BBP has four fixed roles (Planner, Coder, Tester, Reviewer), ten states and two human gates
  (`plan_gate`, `merge_gate`) plus `approved` (merge outside BBP, then `receipt`) and `escalated`.
- The layer check refuses a dependency from one `apps/` family to another, so the conductor cannot
  link `rusty_bbp_host` or orch. It may link `rusty_bbp` (libs) and drive the `bbp` binary, the
  surface BBP keeps stable (as ADR-0013 of rusty_orch concludes).
- A running board holds its store's directory lock, so the conductor talks to the board over its
  HTTP API, never the files.
- orch ADR-0013 (Proposed, unbuilt) plans the same `bbp` driving for orch `Implement` cards.
- The board trusts the `actor`/`role` a caller claims. Fine between cooperating agents with a human
  watching; not fine once an agent runs unattended (a builder could claim `human` and merge).

## Decision
### 1. Shape
A separate process, `mill_conductor`, one per board, one board per application (a factory is a
long-lived company for one application). It is a client of the board's `/api` and of `bbp`; it holds
no state of its own. Every tick it reads the board and each hosted task's `bbp card`, then fixes the
difference. A crash or a missed tick heals on the next one, the same way GitHub sync does.
It lives in the board's family (`crates/apps/rusty_mill_board/conductor`), sharing the card types.

### 2. State mapping
BBP is the source of truth for the task; the board is the source of truth for human intent.

| BBP state | Column | Card carries |
| --- | --- | --- |
| (card in Ready, no handle) | Ready | conductor opens the task, moves to Building, claims |
| (card in Ready, live handle) | Ready | a resume after Blocked; conductor moves to Building, claims |
| `planning`, `build` | Building | |
| `plan_gate` | Building | ask: approve plan (needs Triage) |
| `test` | Qa | |
| `review` | Review | |
| `merge_gate` | Review | ask: approve merge or reject (needs Merge) |
| `approved` | Review | ask: merge outside BBP, then give the receipt (needs Merge) |
| `escalated` | Blocked | ask: resume or cancel, with the reason (needs Triage) |
| `closed` | Done | |
| `cancelled` | Blocked | comment with the reason |

Every move is in the existing transition table. The conductor's role, `conductor`, holds
`Build`, `Qa`, `Review` and `Merge`; it never holds `Triage`. It uses `Merge` only on observing
BBP `closed`, whose receipt a human gave through an ask that required `Merge`.
A person who moves a hosted card to Blocked or Backlog is asking to stop it: the conductor cancels
the task and comments why. Anything else a person does to a hosted card (edit, comment, tag) is left alone.

### 3. Gates as generic asks
The board gains one generic idea, not BBP vocabulary: a card may carry an **ask**.
- `Card.ask: Option<Ask { id, prompt, needs: Capability, refs: Vec<String> }>`; `refs` are opaque to
  the board (the conductor puts the card revision shown and the ids the answer may name, as ADR-0013
  does, so a stale answer is refused rather than applied).
- `card_ask` (set by the conductor) and `card_answer { id, ask_id, text }`. The board checks that the
  answerer holds `ask.needs`, stores the text as a comment, clears the ask and logs `Asked`/`Answered`.
- The conductor relays an answer as a `bbp human` verb, with `--rev` from the ask's refs. A verb naming
  an id not in the refs is refused before `bbp` sees it. `stale_rev` or `wrong_state` becomes a fresh
  ask built from the current gate; a stale answer never silently applies.
- Cards gain `handle: Option<String>` (opaque; `bbp:<dir>/<task>`). The task id is `card-<uuid>`, so a
  retry finds the same task. `issue` stays as is; if a third link kind appears, both become a map.
The UI shows the ask on the card with an answer box, and the verbs the prompt allows.

### 4. Conductor loop (all steps idempotent)
1. List cards. For each Ready card with no handle: `bbp show`/`card` (exists: adopt); write the brief from
   title and body; `bbp open`; `bbp assign` the four roles; start `bbp mod --agents FILE --confine sandbox`
   (its own lock makes a second start a no-op); `card_attach` the handle; move to Building and claim.
   A Ready card **with** a handle is a resume: a person answered an escalation and moved the card out of
   Blocked (their `Triage` decision), so the conductor claims it and moves it to Building. If that task
   is `cancelled` it is not reopened: the conductor opens a fresh attempt (`card-<uuid>-<n>`).
2. For each hosted card not terminal: read the task, compute the column and ask from the table, apply
   the moves and `card_ask`/clear needed. Re-claim after each move (a move clears the claim).
3. For each `Answered` event since the cursor: relay as above.
4. Cancel requests (section 2) and a concurrency cap (`--max-tasks`, default 1) apply before opening.
Decoding follows ADR-0013: a `bbp` exit of 0 can still be a protocol `Rejected`; parse the reply.

### 5. Prerequisites
- **P0, board hardening (blocks unattended use):** bind each role to a credential. A tokens file maps
  bearer token to `{actor, role}`, and the board ignores the `actor`/`role` arguments when it is set. Without
  this, `conductor` is a name anyone can claim.
- **P1, `bbp show`** (the same prerequisite as ADR-0013): the card lacks the gate's proposal, the pending
  request body, and the receipt/cancel reason. Until it lands, the conductor can host a task but the
  prompts for gates are thinner.
- BBP's four fixed roles remain. A factory with eight or nine roles is a BBP change, not a conductor one.

## Consequences
- Humans and agents see one board; BBP's turns, diffs and runs stay in the BBP store, read through the
  handle. No BBP vocabulary enters the board crate.
- Two products will drive `bbp` (this and orch-bbp). That is the second real call site: when both exist,
  extract a `libs` crate for the CLI contract and decoding rather than copying it. Not before.
- Phases, one PR each: **P0** tokens; **A** `ask`/`answer`/`handle` in domain, API and UI; **P1** `bbp show`;
  **B** conductor core as a pure function (table-driven tests of every state), with `Bbp` and `Board`
  ports and fakes; **C** the real adapters, plus an end-to-end test under `--confine none` with scripted
  launchers, as `rusty_bbp_host` does; **D** a proving run through the board, human gates in the UI.

## Alternatives rejected
- **Conductor inside the board process.** The board would depend on `bbp` and a long-lived child
  process for a feature most boards will not use.
- **Route cards through orch goals.** orch's Board and the Mill Board would then both own work-item
  state; orch is goal-file driven and its host is unbuilt. Revisit if orch's `TaskHost` ships first.
- **Give the conductor `Triage`.** Plan approval and resume are the human decisions BBP exists to
  protect (orch ADR-0005: agents never settle decisions alone).
- **A BBP-specific `card_gate` tool.** It would put BBP's verbs in the board's API. A generic ask and
  answer serve any agent, and the conductor translates.

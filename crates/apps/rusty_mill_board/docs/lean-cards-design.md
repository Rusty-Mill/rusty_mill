# Lean cards: design for #1435, #1436 and #1437 (parent #1434)

Status: **approved by Nano, 2026-11-26** (all five decisions as recommended: state line, budgets, comment kinds with the three-line handoff, `card_get` default under the agent profile only (option b), per-credential view profile). Nothing here is built. All three changes are
additive to the MCP contract; none changes the SQLite schema (cards are JSON docs, ADR-0007).

## Problem

Card size and what the board sends overwhelm agent context. Bounded reads (#1426: summaries,
paging, incremental polling) fixed list volume. What remains is card content shape and what an
agent sees by default. Principle (Nano, 2026-11-10): a card says what, who, status and where the
detail lives. Rationale goes in remind-me, specs and logs in the repo, review on GitHub.

Measured on the live board (2026-11-10, `retrieval-scalability-plan.md` §3a; 1,425 cards,
3.87 MB full JSON):

| | Median | p95 | Max |
|---|---:|---:|---:|
| Body | 290 B | 1.7 KB | 28.6 KB |
| Prompt | 654 B | 2.9 KB | 8.9 KB |
| Comments (all of a card's) | 2 B | 3.5 KB | 31.8 KB |

Prompts, comments and bodies are 75% of the bytes. Today's only caps are 32 KiB on the prompt,
small caps on `acceptance` (1,200 B), `scope` (800), `why` (600) and `summary` (320), and none
on title, body or a comment.

## A. State line and content budgets (#1435)

**`state`**: a new optional one-line field on `Card`: current status, in the last writer's words
("review requested, PR #1888", "blocked on Nano: Q25"). At most 200 bytes, no newline. It is
stored in the card doc (`#[serde(default, skip_serializing_if = "String::is_empty")]`, like
`summary`), so old cards and old clients are unaffected. It is distinct from `summary`
(a static one-paragraph description, 320 B): `state` changes as work moves, `summary` rarely.
`state` is also the summary layer the parked StretchText idea (#765) needs.

*Who writes it:* an optional `state` argument on the writes that already touch a card
(`card_move`, `card_comment`, `card_update`), so keeping it current costs no extra call. A
write that leaves it out keeps the old value. `state` appears in the `board_list` summary view
only when non-empty (worst case 1,425 x 200 B = 285 KB, typically far less; the summary view
is 615 KB today).

**Budgets**, enforced on write, refused with a hint ("move the detail to remind-me, the repo or
GitHub and link it"):

| Field | Limit | Why |
|---|---:|---|
| title | 200 chars | same as a milestone title |
| body | 8 KiB | ~5x the p95 (1.7 KB); the largest today is 28.6 KB |
| prompt | 8 KiB (was 32 KiB) | above the p95 and near the largest today (8.9 KB, grandfathered); a prompt is read whole by whoever takes the card |
| one comment | 2 KiB | a comment is a status note; detail belongs in a linked doc |
| state | 200 B | one line |

**Existing oversized cards are grandfathered, never rewritten.** A limit applies only to a write
that changes that field: editing the title of a card with a 28 KB body is not refused, and
nothing is truncated on read. The refusal names the field, the limit and the size sent.

*Open measurement:* the per-comment size distribution was not captured (only per-card totals).
Before the number for comments is final, a read-only pass over the live board reports the
per-comment p95 and max; if p95 exceeds 2 KiB the limit is raised to cover it.

## B. Typed comments and history on request (#1436)

**`kind`** on a comment: one of `note` (default, and how every existing comment reads), `claim`,
`handoff`, `blocker`, `question`, `decision`. An optional `kind` argument on `card_comment`.
Stored with the comment (`#[serde(default)]`); old comments never change.

**A handoff is three short lines: done, next, where the details are.** Enforced for
`kind=handoff` only: at most 3 non-empty lines and 400 bytes; anything else is refused with the
format. Other kinds keep only the 2 KiB cap.

**Resolving a blocker or question.** Today a `Comment` is only `by`, `at_ms`, `text`, so "open"
needs a mechanism. A comment gains an optional `id` (UUIDv7, assigned on write; old comments have
none and are all `note`), returned by `card_comment`. A new optional `resolves: <comment id>`
argument on `card_comment` marks that `blocker` or `question` resolved; it is refused if the id is
not a blocker or question on the same card, and a resolved one is never reopened. A blocker or
question is **open** until a comment names it in `resolves`; nothing is inferred from prose.
Tests: open ones survive the 5-comment window, a resolved one drops out of the default read, a
bad `resolves` is refused, and `history=all` still shows both.

**`card_get` returns less by default**: the card, its `state`, and the last 5 comments plus the
latest `handoff` and every open `blocker` or `question` (so a newer chatty comment never hides
them), with `comments_omitted: n`. `history=all` returns every comment, as today. Older comments
are left out of the default read, never rewritten.

*Compatibility choice (recommended b):*
- a) the new default applies to every caller; the UI and tests pass `history=all`;
- b) the new default applies only under the agent view profile (C below); everyone else keeps
  today's full read until Nano decides otherwise.

(b) follows the S7 precedent: Nano deferred changing a global default to keep clients
compatible. The UI renders `kind` (a small label) and keeps the full thread.

## C. Summary view by default for agent credentials (#1437)

A per-credential **view profile**, not a global default. `credentials.json` gains an optional
`"view": "agent" | "full"` per entry (default `full`; the loader is `deny_unknown_fields`, so
this is a code change, not a config-only one). The profile is orthogonal to role and project
scope: it changes defaults only, never what a credential may see or do.

Under `agent`:
- `board_list` is already `view=summary` by default for everyone (S7, #1635, see
  `changes/2026-11-12-board-list-summary-default.md`), so the profile does not change the view.
  What it adds is paging: today a call with neither `limit` nor `cursor` returns every match, and a
  paged call defaults to 100. Under `agent` an unpaged call is paged with `limit` 50; `full` keeps
  today's behaviour exactly. This makes an agent's list partial by default, so the response
  carries the cursor and the tool description says so;
- `card_get` defaults to the recent-comments read from B (and to nothing else new);
- `card_take` and `card_claim` still return the whole card: the full body and prompt load when
  the agent takes the work.
- Explicit arguments always win: `view=full`, `history=all` and an explicit `limit` are honoured, never refused.

Human and UI credentials stay `full`, so nothing changes for them. `board_list` is **not**
auto-filtered to "ready cards in the agent's lane": hiding cards from a list is surprising, and
`card_take` already does the lane-scoped pull.

Where the profile lives (recommended: the credential entry, because it is per actor and the file
already carries role and projects; an admin Settings screen can edit it later). The alternative,
a Settings-wide switch, would be a second place to look and cannot differ per agent.

## Rollout

Three small PRs, each tested and independently shippable; nothing deploys without Nano's go:
1. `state` + budgets (+ tests: refused over limit with a hint, grandfathered card still
   editable, `state` kept when a write omits it, summary view carries it).
2. Comment `kind` + `card_get` history (+ tests: default read bounds, `history=all`, latest
   handoff and open blocker always present, old comments read as `note`).
3. Credential view profile (+ tests: an agent's unpaged `board_list` is paged at 50 with a cursor,
   `full` still returns every match, explicit `limit` wins, human default unchanged, orthogonal to
   role and scope).

MCP tool descriptions and `docs/connecting-models.md` change in the same PRs. Tool names and
existing arguments are unchanged.

## Decisions for Nano

1. `state`: name, 200 B, last-writer-wins, writable on move/comment/update.
2. Budgets: title 200 chars, body 8 KiB, prompt 8 KiB, comment 2 KiB (final after the
   per-comment measurement), grandfathering as above.
3. Comment kinds and the three-line handoff rule.
4. `card_get` default: last 5 comments + latest handoff + open blocker/question; compatibility
   choice (a) or (b), recommended (b).
5. View profile per credential (recommended) or one global setting.

## Not in this design

Rewriting or compacting existing cards; the global `board_list` default (S7); StretchText UI
(#765); anything in rusty_tick (not used in production).

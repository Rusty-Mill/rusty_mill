# ADR-0006: Observation-only external execution refs

- **Status:** Proposed (2026-11-11), card #1432. It was approved in principle by Nano on 2026-11-10 ([D122](../../../../../docs/projects/let-em-loose/DECISIONS.md)), with defaults in D123. It needs Nano's design approval (D25) before any code. The card schema and MCP API changes below are ask-first items for that approval.

## Context
Some work on the board runs outside Rusty Orch: Codex Cloud threads for Communications (#798) and PythRust, and the GitHub PRs and Actions runs they produce. None of it has an Orch goal, a BBP task or an assignment generation, so it cannot carry an `ExecutionLink`, and inventing one is forbidden (#798). In production, 0 of 1,425 cards have `execution` set. About 63 record Codex Cloud thread and turn ids only in comments, where nothing can query, deduplicate or reconcile them against GitHub.

Rusty Orch is the only executor (D20, D104). Mill Board owns the card and records what Orch did; Orch alone writes `ExecutionLink` (`crates/apps/rusty_orch/ARCHITECTURE.md` § Factory execution boundary). Whatever the board stores about other execution must therefore be a pointer and a last-seen state, never something it acts on.

## Decision

### Schema
The board document (`BoardDoc` in `src/tick.rs`, serialized camelCase) gains an optional `externalRefs`. It is omitted while empty, so every existing document is byte-for-byte unchanged.

```json
"externalRefs": [
  {"provider": "codex-cloud", "ref": "01a1b7d6-f7f1-7717-b1e2-92b4a292c001",
   "url": "https://…", "state": "running", "observedAtMs": 1794085730544, "by": "chatgpt"}
]
```

| Field | Rule |
|---|---|
| `provider` | One of `codex-cloud`, `github-pr`, `github-actions`. Anything else is refused, `orch` and `bbp` included: Orch's ids belong in `ExecutionLink` (D122, D123). A new provider is a code change. |
| `ref` | 1–128 characters of `A-Z a-z 0-9 . _ : / # -`. The provider's own id: a thread id, `owner/repo#123`, a run id. A Codex Cloud turn is not a separate ref; it is the thread's latest state. |
| `url` | Optional, at most 512 bytes, `https://` only. |
| `state` | One of `queued`, `running`, `succeeded`, `failed`, `cancelled`, `unknown`. The first three are live and the next two terminal; `unknown` means the observer could not tell. |
| `observedAtMs` | When the observer saw that state (Unix ms, the observer's clock). |
| `by` | The acting identity, set by the board, never by the caller. |

At most 8 refs per card. With 8 entries at their largest a card adds about 6 KB, inside the existing 64 KiB document preflight (`BoardDoc::checked`), which still refuses an oversized document before any request.

### API
One new MCP and HTTP op, `card_execution_observe {id, provider, ref, state, url?, observed_at_ms}`, which returns the card.
- **Authorization:** the Build capability and the card's project scope, the same as `card_execution_link` (`Board::bind_execution`). A per-model credential's actor is recorded as `by`. A separate `observe` capability would change roles, so it is left as a possible follow-up (D123).
- **Upsert by `(provider, ref)`:**
  - Replaying an identical observation returns the card unchanged and logs nothing. The board's `apply` already treats an unchanged card as an idempotent replay.
  - An older `observed_at_ms` than the stored one is a no-op: the newer state stands, so observations may arrive out of order.
  - The same `observed_at_ms` with a different state is refused as a conflict, so the caller re-observes with a fresh time instead of one write silently winning.
  - A time more than 5 minutes ahead of the board's clock is refused as invalid, so a fast observer clock cannot pin a state that later observations can never replace.
- **The ninth ref:** it replaces the terminal ref with the oldest `observed_at_ms`. If all 8 are live, it is refused. A live observation is never evicted.
- **Event:** a change logs a new `ExecutionObserved` event, so `board_events` pollers and the web UI's incremental refresh pick it up as they do any card change.
- **Reads:**
  - `card_get` and the full `board_list` view carry `external_refs`.
  - Summaries gain two flags (D123): `has_execution` (an `ExecutionLink` is set) and `has_external_refs` (any ref is set). A single flag for both is rejected, because a reader could take an observation for an Orch execution.
  - The card dialog lists the refs, linking each `url`.

There is no op to delete a ref. Eviction bounds the list, and an observation is history.

### The invariant
Nothing in Mill Board reads `external_refs` to decide anything. `card_take` never skips, reorders or refuses a card because of its refs, since choosing a card is dispatch (D122). Moves, claims, `card_execution_link` and `card_execution_recover` ignore them. Refs take no part in the generation compare-and-set, and Orch never reads them as recovery state.

**Guard:** one test runs `card_take`, `card_move`, `card_claim`, `card_execution_link` and `card_execution_recover` on two otherwise identical boards, one whose cards carry refs in every state (including `running`). It asserts the same results and the same events, apart from the refs themselves.

A card may carry an `ExecutionLink` and refs at once, for example a `github-pr` ref next to an Orch link. That cannot be prevented anyway, because a link can be bound after refs exist, and since nothing reads refs it does no harm (D123).

### Retention
Refs stay on Done and archived cards, and restore keeps them: they are the record of what ran. The cap of 8 is the only eviction.

### Compatibility and rollback
- **A new binary reading an old document:** `externalRefs` defaults to empty.
- **An old binary reading a new document:** `BoardDoc` has no `deny_unknown_fields`, so it reads the card and ignores the refs. But it rewrites the whole document on that card's next write, which drops them.
- To make rollback lossless, the implementation lands in two PRs:
  1. `BoardDoc` keeps fields it does not know (`#[serde(flatten)]` into a JSON map) and writes them back unchanged. This is generic and adds no new field. It is deployed first.
  2. `external_refs` itself.

  Rolling back from 2 to 1 then loses nothing. Rolling back below 1 drops refs from a card's document on that card's next write. That is accepted and stated in the release notes.
- **MCP schema:** a new tool, a new optional field on the card, and two summary booleans. Existing calls are unchanged. Connectors pick up the new tool after a reconnect.

### Consumers and backfill
- #798 Communications and the PythRust cloud cards call `card_execution_observe` when they launch, resume or finish a Codex Cloud thread, and when a PR they opened changes state. #798's reconciliation then compares refs against GitHub, not comment text.
- **Backfill:**
  - Comments are never rewritten or parsed by the board.
  - A coordinator that still holds the ids, such as #798's thread `01a1b7d6-…`, may observe them once, with the `observed_at_ms` it actually saw.
  - Ids that live only in old comments stay there. A bulk parse of comment text would fabricate states and times, and the absence of a ref is the honest record.

## Consequences
- Non-Orch execution becomes queryable and reconcilable without a second orchestration engine inside the board.
- One more op and one more event kind. The document stays well under its size limit, and S1 summaries stay small (two booleans).
- The eviction rule and the provider set are code, so a new provider or a larger cap is a reviewed change.
- Rejected:
  - Keeping ids in comments only (status quo, unqueryable).
  - Running these workers under Orch so that they get a real `ExecutionLink`. That is correct long-term but blocked on deployment (#1092, and #798's open host question). Refs do not preclude it: a card later run by Orch gets a link, and its refs remain history.

---
category: Added
changelog: Mill Board cards record observation-only refs to Codex Cloud threads, GitHub PRs and Actions runs, written by the new `card_execution_observe` tool (ADR-0006).
---
## 2026-11-12 - Mill Board external execution refs

- **Added:** `card_execution_observe {id, provider, ref, state, url?, observed_at_ms}` (MCP and HTTP). It needs `build` and the card's project, and records the acting identity as `by`. It upserts by provider and ref. A replay or an older observation changes nothing, the same time with a different state is refused, and a time more than 5 minutes ahead of the board's clock is invalid. A card holds at most 8 refs: a new one replaces the finished ref seen longest ago, and it is refused when none has finished. Each change logs an `execution_observed` event.
- **Added:** cards carry `external_refs` in `card_get`, `card_take` and the full `board_list` view, and the card dialog lists them, linking each url. Summaries are unchanged (D126).
- **Invariant:** nothing on the board reads refs to decide anything. A test runs take, move, claim, execution link and recover on boards with and without refs in every state, and requires the same results and events.
- **Rollback:** deploy the S5a release (#1624) first. Rolling back to it keeps refs. Rolling back to any older binary drops a card's refs on that card's next write.

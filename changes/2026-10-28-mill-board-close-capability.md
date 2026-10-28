---
category: Added
changelog: Mill Board: a `close` capability lets a role record a building or QA card whose PR already merged as Done.
---
## 2026-10-28 - Mill Board: record already-merged work as Done

- **Added:** a `close` capability on `rusty_mill_board`. A role holding it may move a card from Building or QA straight to Done, with the card's claim, when the work already merged on the forge. The default `builder` role holds it. Before, only Review -> Done (merge) existed, so finished agent work sat in Building until a person dragged it.
- **Added:** `card_add` takes `done: true` (needs `close`) to record merged work that never had a card; on rusty_tick its task is created closed.
- **Changed:** `close` never makes a card someone's work: `card_take` and the actionable test ignore it, so builders are not handed QA cards.
- **Known limitations:** the board does not check that the PR merged; the agent's comment is the record. A board run with its own `roles.json` must add `close` to a role in Settings to use it.

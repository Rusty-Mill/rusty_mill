---
category: Changed
changelog: Mill Board's `board_list` now returns card summaries by default; pass `view: "full"` for whole cards (S7).
---
## 2026-11-12 - Mill Board `board_list` defaults to summaries

- **Changed:** `board_list` without `view` now returns summaries: id, number, rev, title, project, column, priority, claim, tags and times. Body, prompt, links and comments are left out. On the live board a full listing is about 6.3 MB over MCP; a summary listing is about 0.65 MB and about 2.5 times faster through the tunnel (S0, #1428).
- **Migration for callers:** pass `view: "full"` to keep whole cards, or `card_get` one card. In this repository the web UI and mill-runner now pass it. Outside callers that read `body`, `prompt` or `comments` from `board_list` (ChatGPT, Dottius, super-board prompts) must add `view: "full"`. MCP clients see the new default in the tool description after a reconnect.
- **Unchanged:** paging, filters, scope, `card_get`, `card_take`.

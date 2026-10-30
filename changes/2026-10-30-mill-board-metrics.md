---
category: Added
changelog: Mill Board shows board metrics (work in progress, weekly flow, lead time, idle cards) derived from its cards, in the web UI and as the board_metrics tool.
---
## 2026-10-30 - Mill Board metrics

- **Added:** a Metrics toggle in the Mill Board web UI, beside Roadmap, for the chosen project, else the chosen
  repository, else the whole board: open, in progress, blocked, shipped in 30 days and created-to-shipped lead
  time (median and 85th percentile); cards added and shipped per week for 8 weeks; open cards by column,
  priority and project; in-progress claims per agent; and the 5 open cards past backlog idle longest. It
  follows board changes live and is remembered in the browser. The same numbers are the `board_metrics` tool
  (MCP and `POST /api/board_metrics`), scoped like `board_list`. No rusty_tick change.
- **Known limitations:** no time-in-column or cycle time: the board keeps column moves only in memory, so
  lead time runs from creation to Done. Weeks are rolling 7-day windows ending now, not calendar weeks.

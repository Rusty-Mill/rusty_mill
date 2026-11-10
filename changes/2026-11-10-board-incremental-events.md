---
category: Changed
changelog: Mill Board `board_events` polls read only what changed in rusty_tick, see comments made there, and tell a poller to resync after a board restart.
---
## 2026-11-10 - Mill Board incremental events

- **Changed:** a `board_events` poll reads tasks changed since the last poll and comments newer than the last seen, not the whole board: an idle poll on a production-sized board drops from 1.24 MB to 1.4 KB of tick traffic. Every five minutes a poll reads the whole agent listing, so a change a delta read missed is still caught.
- **Added:** a comment made in rusty_tick, not through the board, is now a `commented` event.
- **Added:** `board_events` answers `resync: true` when the `after` cursor is one the board does not hold (it restarted since); the events are then the log from its start. The web UI re-reads the board on a resync.
- **Known limitations:** the web UI still re-reads the whole board when any event arrives; a refresh of only the changed cards follows separately.

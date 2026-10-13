---
category: Added
changelog: **BBP shadow log and `bbp shadow`.** Every host process that opens a driver appends the shadow records of its grants to a per-task log, once per `rev`, and `bbp shadow` tallies how often an event-driven rule would have picked another role. Nothing is decided by it.
---
## 2026-10-13 - BBP host: shadow log and `bbp shadow`

- **Added:** `Driver::with_shadow_sink` and `ShadowSink` (rusty_bbp): a callback given the records of each successful dispatch that made any; it cannot fail or change the dispatch. `Shadow`, `ShadowKind` and `WakeReason` are now serializable.
- **Added (host):** `rusty_bbp_host::shadow`. `open_driver` installs a sink that appends the records to `<dir>/shadow/<sha256(task)>.jsonl`, one JSON record per line, skipping any `rev` already in the file, so every process that opens a driver (`mod`, `mcp`, the CLI, the runner) contributes to one log. A failure to log is reported on stderr and never fails the command. `bbp shadow --dir D --task T` prints the tally as JSON: records, unreadable lines, and agree / fallback / disagree / explicit by the state the grant was made in. A torn or foreign line is counted, never skipped silently.
- **Unchanged:** the event log, every grant and every existing test. The shadow log is a separate file and nothing reads it back into a decision.
- **Known limitations:** shadow records exist only for a turn-based task with one live turn. Two processes appending concurrently can write the same `rev` only if both dispatched the same grant, which the store's revision check forbids. No event-driven grant exists.

---
category: Changed
changelog: remind-me's folder watcher captures Claude Code sessions into the hook's per-session capture (backfilling older ones, refreshing grown ones in place) after an idle period, instead of importing them a second time.
---
## 2026-10-27 - Watched Claude Code sessions converge on the hook's capture

- **Changed:** with `REMIND_ME_WATCH_CLAUDE_PROJECTS=1`, the folder watcher no longer chat-imports `~/.claude/projects/` transcripts. Each `<project>/<session>.jsonl` goes through the same per-session capture the plugin hooks write, keyed by session id. The first scan backfills every older session the hooks never saw; a session the hook already captured at that length is left alone; one that grew past the hook's copy is refreshed in place. Subagent transcripts are skipped.
- **Added:** `REMIND_ME_WATCH_IDLE`, the seconds a file must have been idle before the watcher considers it. Defaults to 600 for Claude Code sessions, so a live session paused between turns is not captured on every pause, and 0 for `REMIND_ME_WATCH_DIRS`, which keeps their behaviour. `remind_me_watch_status` reports `idle_seconds` and `ingest` (`files` or `sessions`).
- **Fixed:** transcript captures in one process are serialized: the hook, the MCP tool, the backfill and the watcher take one lock across the lookup, the dialog and summary writes and the archive, so two first captures never make two pairs. The watcher and the backfill decide under that lock whether to write, so a stale watcher reading never replaces a newer hook capture. A project folder that cannot be listed is reported and no longer hides the other projects from `import-sessions` or the watcher.
- **Known limitations:** a watcher-written capture is stamped `written_by: hook` like a backfill, and only with the cwd and branch the transcript records.

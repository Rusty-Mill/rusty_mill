---
category: Added
changelog: rusty-remind-me `import-sessions DIR` backfills older Claude Code session transcripts, one capture per session, stamped with each transcript's own cwd.
---
## 2026-10-26 - Backfill Claude Code sessions into remind-me

- **Added:** `rusty-remind-me import-sessions DIR` captures every `DIR/*/*.jsonl` Claude Code session not captured yet, through the same path the transcript hook uses: one dialog and summary pair per session, keyed by the transcript's session id, with `reason: backfill`. Sessions already captured are skipped, so a re-run or an overlap with the hook writes nothing twice. It prints `{"imported", "skipped", "empty", "failed"}`; an unreadable file is listed in `failed` and the walk continues.
- **Fixed:** a transcript cwd that is not a directory on this host (a Windows `C:\...` path read on Linux) now names its project by its last segment, and the capture no longer carries the host process's git remote, branch and sha.
- **Known limitations:** captures are dated at import time, not at the session's own time. Subagent transcripts (deeper than `DIR/*/`) are not imported.

---
category: Added
changelog: rusty-remind-me `import-sessions DIR` backfills older Claude Code session transcripts, one capture per session, stamped only with where each transcript says it ran.
---
## 2026-10-26 - Backfill Claude Code sessions into remind-me

- **Added:** `rusty-remind-me import-sessions DIR` captures every `DIR/*/*.jsonl` Claude Code session not captured yet, through the transcript capture the hook uses (one dialog and summary pair per session, scrubbed raw archive included), keyed by the transcript's trimmed session id, with `reason: backfill`. Sessions already captured are skipped, so a re-run or an overlap with the hook writes nothing twice. It prints `{"imported", "skipped", "empty", "failed"}`; an unreadable file is listed in `failed` and the walk continues. A backfilled session is stamped with the transcript's own cwd and branch and the project that cwd names (either separator, so a Windows `C:\...` path works on Linux), and never with this host's git.
- **Fixed:** a transcript capture given its caller's context (`remind_me_capture_transcript`, and the backfill) no longer fills the columns that context leaves unknown from the store process's own project, cwd and git: a `git_remote: None` used to come back as the server's remote.
- **Known limitations:** captures are dated at import time, not at the session's own time. Subagent transcripts (deeper than `DIR/*/`) are not imported.

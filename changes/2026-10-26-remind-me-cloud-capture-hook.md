---
category: Added
changelog: rusty_remind_me: a repo hook captures claude.ai Projects (cloud) sessions in remind-me through the remote connector's remind_me_capture_transcript (ADR-0029).
---
## 2026-10-26 - remind-me: capture cloud sessions through a repo hook

- **Added:** `.claude/hooks/remind-me-capture.py`, registered on `Stop` and `PreCompact` in `.claude/settings.json`. In a claude.ai Projects session, which has no plugin, no binary and a transcript that is lost with its container, it sends the transcript to the remote connector's `remind_me_capture_transcript`. The server then keeps one capture per session, replaced in place. The hook also sends the session's project, branch, commit, cwd and `origin` remote, with any credentials stripped from the remote.
- **Added:** setup notes under "Cloud sessions" in the rusty_remind_me README.
- **Known limitations:** the hook runs only when `CLAUDE_CODE_REMOTE=true` and both `REMIND_ME_URL` and `REMIND_ME_TOKEN` are set. The cloud environment must allow the connector's host, and the connector must run the `full` tool profile. A transcript over 3 MiB once JSON-escaped is sent as its newest whole lines. `SessionEnd` is not used, because a reclaimed container may never fire it.

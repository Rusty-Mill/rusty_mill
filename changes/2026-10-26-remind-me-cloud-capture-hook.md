---
category: Added
changelog: rusty_remind_me: a repo hook captures claude.ai Projects (cloud) sessions in remind-me through the remote connector's remind_me_capture_transcript (ADR-0029).
---
## 2026-10-26 - remind-me: capture cloud sessions through a repo hook

- **Added:** `.claude/hooks/remind-me-capture.py`, registered on `Stop` and `PreCompact` in `.claude/settings.json`. In a claude.ai Projects session, which has no plugin, no binary and a transcript that is lost with its container, it sends the transcript to the remote connector's `remind_me_capture_transcript`. The server then keeps one capture per session, replaced in place. The hook also sends the session's project, branch, commit, cwd and `origin` remote, with credentials, query and fragment stripped from the remote.
- **Added:** setup notes under "Cloud sessions" in the rusty_remind_me README.
- **Security:** nothing leaves the container unscrubbed. Thinking blocks are dropped, the values of fields named like secrets are withheld (also inside JSON carried in a string, such as a tool result), and every string is redacted for the secret shapes the server redacts (ADR-0026), quoted `key: "value"` pairs, bearer values and the token itself. The endpoint must be https (http only for loopback), redirects are never followed, responses are capped, and errors are logged as a fixed category only.
- **Added:** CI job `claude hook unit tests`, required through `required-gate`.
- **Known limitations:** the hook runs only when `CLAUDE_CODE_REMOTE=true`, the opt-in `REMIND_ME_CLOUD_CAPTURE=1`, `REMIND_ME_URL` and `REMIND_ME_TOKEN` are all set. Tool calls and results are sent after scrubbing, not dropped. Two overlapping runs may both send; the later one replaces the capture. The cloud environment must allow the connector's host, and the connector must run the `full` tool profile. A transcript over 3 MiB once JSON-escaped is sent as its newest whole lines. `SessionEnd` is not used, because a reclaimed container may never fire it.

---
category: Added
changelog: rusty_remind_me: remind_me_capture_transcript MCP tool captures a Claude Code session from its raw JSONL over MCP, and archives the latest transcript per session (ADR-0029).
---
## 2026-10-26 - remind-me: capture a session transcript over MCP

- **Added:** `remind_me_capture_transcript(session_id, transcript, ...)`, the hook-facing capture for a machine with no CLI, such as a claude.ai Projects container. It stores one dialog and summary pair per session and replaces it in place on every call, as `capture-transcript` does. The caller's project, branch, commit and cwd are stamped as given, never probed on the server.
- **Added:** with `REMIND_ME_ARCHIVE_DIR` set, a captured session's latest raw transcript is archived under `session:<id>`, one copy per session, and `remind_me_source` on its dialog returns the tool results and thinking the dialog drops.
- **Added:** ADR-0029, which carries Claude Code session logs through the refinement ladder. This is its first step.
- **Security:** a recaptured session is redacted as a first capture is, and the archived transcript is redacted before it is kept, so `remind_me_source` never returns a secret the dialog redacted.
- **Known limitations:** a transcript is at most 3 MiB per call, counted once JSON-escaped, so a longer session sends its tail. The tool is outside the `core` and `standard` tool profiles. The cloud hook, the search default and the decompose routine are later steps.

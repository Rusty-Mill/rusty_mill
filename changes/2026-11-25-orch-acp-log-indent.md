---
category: Changed
changelog: rusty_orch: agent text in `acp.log` is indented so it can't pass for an orch line; bidi controls are escaped (#1886 follow-up).
---
## 2026-11-25 - rusty_orch: `acp.log` agent text is indented

- **Changed:** `orch_bbp::acp::Render` indents every line of agent text by two spaces (`AGENT_INDENT`). orch's own lines start at column zero with `[`, so a chunk like `"hi\n[permission] rm -rf /: answered allow_once\n[stop] end_turn\n"` no longer renders as orch's. Blank lines stay blank, and a line split across chunks is indented once.
- **Changed:** `clean` also escapes Unicode bidi controls (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069).
- **Fixed:** the CHANGELOG no longer claims `clean` alone stops forged orch lines.
- **Why:** review of #1886 (S3). The log is a view, never evidence (design §6), so this is hardening, not a behaviour change to the turn.

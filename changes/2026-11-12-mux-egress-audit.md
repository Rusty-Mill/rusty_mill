---
category: Added
changelog: The mux binaries were audited for unrequested network calls and none were found; .github/scripts/mux_egress_audit.sh repeats the check.
---
## 2026-11-12 - Mux egress audit (C09)

- **Added:** `.github/scripts/mux_egress_audit.sh`.
  - **What it runs:** the mux server, ctl, mcp and TUI through a scripted session under `strace`.
  - **When it fails:** if any of them opens a network socket or reaches an IP address. On a clean session only Unix sockets are used.
- **Added:** `docs/projects/mux/EGRESS-AUDIT.md`. It lists every network call site and the user action that triggers it. There are no update checks, telemetry or account calls. C09 is now marked done as a baseline, and the Phase 5 exit repeats the audit on the release build.

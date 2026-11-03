---
category: Changed
changelog: Record the rusty_mux Windows manual check: R11 confirmed, R13 checked, and the Ctrl-] prefix found not working in Windows Terminal.
---
## 2026-10-28 - rusty_mux Windows manual check

- **Changed:** `docs/projects/mux/WINDOWS-MANUAL-CHECK-2026-10-28.md` records the manual check on Windows 10.0.26300. CUJ 2, C30, C31, C33 and CUJ 4 passed. R11 is confirmed (`start ping` outlives a closed block), and R13 is checked (an elevated client of an unelevated server works). `RISKS.md` is updated for R10, R11 and R13.
- **Known limitations:** The `Ctrl-]` prefix was not caught in Windows Terminal (`Ctrl-]` `d` echoed `^]d`), so detach and the C12 layout commands could not be checked. No fix is included. CUJ 3 (second account, R10) was not run, so the DACL layer is still unconfirmed. Builds older than 26100 were not checked.

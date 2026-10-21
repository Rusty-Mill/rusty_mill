---
category: Security
changelog: Update rusty_tick's web tooling to Vite 6.4.4 for upstream development-server path access hardening.
---
## 2026-10-21 - rusty_tick Vite development-server hardening

- **Security:** Update `rusty_tick/web` from Vite 6.4.3 to 6.4.4,
  incorporating upstream fixes that harden transform access checks and safe
  module-path tracking when the development server is exposed to untrusted
  requests.
- **Scope:** This conditional exposure applies to the upstream Vite development
  server; this change does not identify a production application exploit.

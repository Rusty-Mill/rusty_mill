---
category: Security
changelog: "**rusty_fair_play/web:** update Vite from 6.4.3 to 6.4.4 for upstream development-server path-access hardening."
---
## 2026-10-21 - rusty_fair_play Vite development-server hardening

- **Security:** update the `rusty_fair_play/web` development dependency Vite
  from 6.4.3 to 6.4.4, incorporating upstream fixes that tighten transform
  access checks and safe-module path handling in the development server.
- **Scope:** the hardening applies when the Vite development server is used;
  this update does not indicate a demonstrated exploit in production builds.

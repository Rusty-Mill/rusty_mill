---
category: Changed
changelog: The Ai Env Manager desktop UI has a cleaner, modern look and a search box.
---
## 2026-11-14 - Ai Env Manager GUI: UI polish

- **Changed:** restyled the static UI to the design boards' tokens (dark palette, state dots instead of pill badges, uppercase section labels, 44px targets, focus rings, reduced-motion support). The plan dialog now groups "Will apply" and "Refused, left untouched". Added a client-side search that only narrows the visible rows.
- **Known limitations:** system font stacks only (the CSP allows no remote fonts). The boards' sidebar, kind filters, per-tool toggles and file editor are not built; the backend has no commands for them.

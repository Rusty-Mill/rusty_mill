---
category: Fixed
changelog: The Ai Env Manager theme button now works when browser storage is blocked.
---
## 2026-11-14 - Ai Env Manager GUI: theme toggle with blocked storage

- **Fixed:** the chosen theme is kept in memory and persisted only when storage allows it. Before, with storage blocked and a light system theme, two clicks both chose dark. Found in the review of #1696.
- **Known limitations:** no automated UI test; checked with Playwright (storage get/set throw, light system, clicks give dark then light).

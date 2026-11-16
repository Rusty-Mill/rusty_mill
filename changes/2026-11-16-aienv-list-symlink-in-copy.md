---
category: Fixed
changelog: aienv list and the GUI list no longer abort when a managed copy contains a symlink.
---
## 2026-11-16 - Ai Env Manager: a symlink inside a copy no longer breaks the listing

- **Fixed:** a symlink added inside a marked copy made `observe` fail, so `aienv list` (and the GUI list) aborted and hid every other skill. The copy now reads as edited, which also keeps it from being replaced or removed.
- **Known limitations:** `edit` and `push` still refuse such a copy.

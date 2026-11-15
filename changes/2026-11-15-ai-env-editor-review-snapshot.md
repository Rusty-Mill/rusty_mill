---
category: Fixed
changelog: The Ai Env Manager editor saves exactly the text its diff showed, even if you type while the preview loads.
---
## 2026-11-15 - Ai Env Manager: editor saves the reviewed text

- **Fixed:** the editor sent the textarea's current value on Confirm, so text typed while the preview request was out could be saved without being reviewed (P2 from the review of #1714). Review now snapshots entry, tool, path, text and hash and freezes the box while the request is out; Confirm saves exactly that snapshot, going back to editing drops it, and a preview that returns after the user switched files is ignored.
- **Known limitations:** the regression is a standalone script, `crates/apps/ai_env_manager/gui/tests/ui/review-snapshot.cjs` (`node tests/ui/review-snapshot.cjs` from `gui/`, needs a resolvable `playwright`); CI does not run it. The stale-preview guard also compares entry and tool.

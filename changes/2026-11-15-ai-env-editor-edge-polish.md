---
category: Added
changelog: The Ai Env Manager editor asks before dropping unsaved text, offers "Discard and reload" (also the way out of "changed on disk"), and says when one save reaches several tools.
---
## 2026-11-15 - Ai Env Manager: editor edge polish (slice 5)

- **Added:** an in-dialog prompt (Keep editing / Discard changes) before switching files, Close or Esc would drop unsaved text; "Discard and reload" in edit and review, which re-reads the file and is the way out of a "changed on disk" refusal; a note on a shared source file ("Used by claude, codex: one save changes all of them") in the file note and the review footer.
- **Fixed:** the footer kept "Saved." into the next edit and the next skill; it now resets when a skill is opened and when Edit starts.
- **Changed:** `ui/` only; no Rust, CSP or dependency change. The "edited" state after saving a marked copy already shows in the list because the page refreshes after each save; no extra wiring was needed.
- **Known limitations:** the checks are standalone scripts, `crates/apps/ai_env_manager/gui/tests/ui/editor-flow.cjs` and `review-snapshot.cjs` (`node tests/ui/<name>` from `gui/`, needs a resolvable `playwright`); CI does not run them. The shared-file note uses the tools shown enabled in the list when the dialog opened.

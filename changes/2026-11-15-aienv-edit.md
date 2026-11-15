---
category: Added
changelog: aienv edit changes one text file of a skill from the command line, with a diff and a confirmation before saving.
---
## 2026-11-15 - aienv edit

- **Added:** `aienv edit ID FILE --skills DIR... [--tool ID] [--from PATH|-] [--yes]`. The new text comes from `--from` (a file, or `-` for stdin), else from `$VISUAL` / `$EDITOR` run on a temporary copy. It prints the diff and writes only with `--yes`; after the editor it asks `Save? [y/N]` on a terminal, and with no terminal and no `--yes` it exits 2 before opening the editor. It uses the same rules and save as the GUI editor (`ai_env_app`), so foreign copies, the marker file, paths outside the skill, and files changed since they were read are refused.
- **Known limitations:** one file per run; existing text files only (no create, rename or delete); Windows saves stay in place and Unix saves drop extended ACLs, as in the GUI.

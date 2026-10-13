---
category: Fixed
changelog: Library review fixes - `backup-before-edit` writes to the home directory and skips secret-looking files, skills export their whole `SKILL.md`, flagged entries stay disabled in the detail view, and selected filters stay visible.
---
## 2026-10-13 - Library review fixes (`library/`)

- **Fixed:** `backup-before-edit` copied every edited file into `.claude/backups/` inside the checkout, where `git add .` could pick up a copy of an ignored `.env`. It now writes to `$HOME/.claude/backups/`, skips secret-looking names (`.env*`, `*.pem`, `*.key`, `id_*`, `credentials*`) and writes nothing when no home directory is known.
- **Fixed:** skill Copy/Download exported only the body, dropping name, description, tags and source. Skills now export the whole `SKILL.md` (downloads are named `SKILL.md`); prompts and CLAUDE.md/AGENTS.md sections stay body-only.
- **Fixed:** a flagged entry's Copy was disabled on the card but enabled in the detail view. The detail view now shows the note and disables Copy and Download.
- **Fixed:** a selected filter chip vanished when its count dropped to zero, leaving empty results with no visible filter. It now stays, and an empty result offers "Clear search and filters".
- **Verified (run):** `python3 -m unittest` (9 tests, incl. a skill export round trip), `claude plugin test` on the mod (10 tests: outside-project path, five secret names, no home dir), and a headless Chromium pass over the download file name and content, flagged to normal to flagged navigation, and the zero-count chip and reset.
- **Known limitations:** backups are plain copies in the home directory (no mode bits set, no pruning) and the secret-name list is a heuristic. The mod is untested in a live session. CI does not run `library/` tests; they are author-run.

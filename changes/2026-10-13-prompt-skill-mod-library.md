---
category: Added
changelog: `library/`, a prompt, skill, mod, CLAUDE.md and AGENTS.md library as plain files with a stdlib builder and an offline browser UI; every entry records its source.
---
## 2026-10-13 - Prompt, skill and mod library (`library/`)

- **Added:** `library/` holds five kinds of entry as plain files: prompts, skills (`SKILL.md`), mods (Claude Code plugins), CLAUDE.md sections and AGENTS.md sections/templates. `python3 library/build.py` writes one self-contained `dist/index.html` (search, kind and tag filters, copy, download). No new dependency; stdlib only.
- **Added:** every entry must record where it came from (`source`, or `repository` in a mod's `plugin.json`); the build fails without it and the UI shows it.
- **Seeded:** 13 entries from existing repo prompts, the nexus builtin skills, a `CLAUDE.md` section from `rusty_remind_me`, and one starter mod, `backup-before-edit`.
- **Verified (run):** `python3 -m unittest` (6 tests), `claude plugin validate` and `claude plugin test` on the mod (3 tests, in-memory fs), and a headless Chromium pass over search, filter, detail and back.
- **Known limitations:** the mod was exercised in the test kit, not in a live session. The five nexus skills carry no recorded licence (nexus is frozen). Not registered in `.claude-plugin/marketplace.json`. The UI was not checked on mobile widths or with a screen reader.

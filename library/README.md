# library

Prompts, skills, and mods as plain files, browsable in one offline page.

| Kind | Path | Format |
| --- | --- | --- |
| prompt | `prompts/<name>.md` | frontmatter: `name`, `description`, `tags: [a, b]` |
| skill | `skills/<name>/SKILL.md` | same frontmatter; drop-in for `~/.claude/skills/` |
| mod | `mods/<name>/` | Claude Code plugin; tags come from `keywords` in `plugin.json` |
| claude-md | `claude-md/<name>.md` | a CLAUDE.md section; same frontmatter |
| agents-md | `agents-md/<name>.md` | an AGENTS.md section or template; same frontmatter |

```
python3 build.py              # -> dist/index.html (open directly, no server)
python3 -m unittest           # parser + seed checks
claude plugin validate mods/<name>
claude plugin test mods/<name>   # backup-before-edit: 10 tests
```

Every entry must record where it came from: `source:` in frontmatter (mods: `repository` in `plugin.json`) - a repo path, URL or "original". The UI shows it.

Optional frontmatter: `for:` (target tool; defaults per kind), `status:` (greys the card, disables Copy, shows the note - use for an entry that needs a fix), `featured: true` (sorts first when nothing is filtered).

Add an entry by dropping a file in; the build fails on a missing name/description or duplicate.

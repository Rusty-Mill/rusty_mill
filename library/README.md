# library

Prompts, skills, and mods as plain files, browsable in one offline page.

| Kind | Path | Format |
| --- | --- | --- |
| prompt | `prompts/<name>.md` | frontmatter: `name`, `description`, `tags: [a, b]` |
| skill | `skills/<name>/SKILL.md` | same frontmatter; drop-in for `~/.claude/skills/` |
| mod | `mods/<name>/` | Claude Code plugin; tags come from `keywords` in `plugin.json` |

```
python3 build.py              # -> dist/index.html (open directly, no server)
python3 -m unittest           # parser + seed checks
claude plugin validate mods/<name>
```

Add an entry by dropping a file in; the build fails on a missing name/description or duplicate.

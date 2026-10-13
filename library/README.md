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

## Sources: entries that live in other repos

Skills, mods and prompts you already maintain elsewhere are indexed in place, not copied. List the repo in `sources.json`:

```json
[{ "name": "my_claude_mods", "repo": "baileyrd/my_claude_mods", "ref": "<commit SHA, tag or branch>" }]
```

```
python3 build.py --sync      # fetch each source (git, https only, shallow) into .cache/, then build
python3 build.py             # offline: index what is already in .cache; an unsynced source is a warning
python3 build.py --strict    # fail on any warning
```

What is indexed: `SKILL.md` folders, plugin folders (a `.claude-plugin/plugin.json`), and `prompts/*.md` at the repo root. Their `source` is computed as `github.com/<repo>@<sha>:<path>` and the repo name becomes a tag; "By source" groups by `repo@sha`. Pin `ref` to a full SHA for reproducible builds and bump it to update.

Rules for remote content: it is only read as text, never executed. Symlinks are not followed, `.git`/`node_modules`/`tests`/`types` are skipped, files are cut at 64KB and a mod at 256KB (with a marker), and a source cannot set `featured` or `status` on its entries. `repo` and `ref` are validated, and a clash with another entry fails the build. Private repos work if `git` on your machine can read them.

Add an entry by dropping a file in; the build fails on a missing name/description or duplicate.

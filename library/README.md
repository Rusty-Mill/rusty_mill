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
claude plugin test mods/<name>   # backup-before-edit: 18 tests
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

Rules for remote content:

- It is only read as text, never executed. Symlinks are not followed: a file must resolve inside the checkout, and every file read for a mod must resolve inside the mod folder (so a symlinked `hooks` directory is refused). `.git`, `node_modules`, `tests`, `types` are skipped; files are cut at 64KB and a mod at 256KB, with a marker.
- Everything taken from a source is type-checked: `name`, `description`, `for` are non-empty strings, `tags` a list of at most 20 short strings, none with control characters, and the path likewise. A malformed entry is skipped with a warning (`--strict` fails). A source cannot set `featured` or `status`.
- `repo`, `ref` and `name` are validated, and a clash with another entry fails the build.
- **Install commands** for a mod are built in Python (never in the page), quoted as data for bash/zsh and PowerShell, and fetch exactly the commit the page indexed (`git fetch` of the full SHA, then checkout), not the default branch, into a new directory: the command refuses an existing one, so a stale or edited checkout is never launched, and it stops at the first failing step (PowerShell included). A path that cannot be quoted safely for every shell (control characters, line separators, `"`, and the typographic quotes U+2018-U+201E, which PowerShell treats as quote characters) gets no command and a warning. Mods run unsandboxed once launched: read the hooks first.
- **The cache is bound to its origin.** `--sync` records the repo and ref in the checkout; a build refuses a cache that does not match `sources.json` (a different repo or ref, or a pinned SHA it is not at) or whose worktree differs from its commit (modified, untracked or ignored files), with a warning, or a failure under `--strict`. `--sync` fetches into a fresh directory and replaces `.cache/<name>` only if it is an untouched checkout made by a previous sync; a symlink, a foreign directory or a checkout with local changes is refused and left alone (move or delete it, then re-run). Nothing is ever cleaned in place.

Private repos work if `git` on your machine can read them.

Add an entry by dropping a file in; the build fails on a missing name/description or duplicate.

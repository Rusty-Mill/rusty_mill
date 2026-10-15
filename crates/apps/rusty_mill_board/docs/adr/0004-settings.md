# ADR-0004: Settings: managing agents, repositories, roles and GitHub from the UI

- **Status:** Accepted (2026-10-15)

## Context
Agents (credentials), projects, roles and the GitHub sync were server flags and hand-written JSON files.
Adding an agent meant editing a file and restarting; nothing could be changed from the board itself.

## Decision
- **The files stay the source of truth.** `--settings-dir DIR` (or one flag per file) names `identities.json`,
  `projects.json`, `roles.json` and `github.json`. The settings view reads and rewrites those files in place,
  so existing setups keep working, files stay human-readable and diffable, and with no files configured the
  view is read-only and says how to enable it.
- **Validate, write, then apply.** Every change is checked, written atomically (`rusty_atomic_file`; files
  with secrets owner-only), and only then swapped into the running board: credentials, roles and projects
  take effect at once, the GitHub sync is restarted. A write that fails changes nothing.
- **A new `Admin` capability** gates every settings operation; the default `human` role holds it. On a board
  with no per-model credentials the caller's claimed role is trusted, as for every other action.
- **They are not MCP tools.** The operations are `POST /api/settings_*` only: a model should not mint credentials.
- **Tokens.** A token is shown once, when it is made or rotated, and never listed. The first start with a
  settings directory creates `identities.json` with an `admin` credential and prints its token once.
- **No lock-out.** A change is refused if it would leave no agent holding a role with `Admin`, drop a role an
  agent has, drop a project that still has cards or that an agent is limited to, or leave no role with `Admin`.
- **GitHub token** is kept in `github.json` (owner-only), is write-only from the UI (a blank field keeps the
  stored one) and is never returned.

## Consequences
- Credentials are stored in plain text in an owner-only file, as before: the file is the secret.
- Roles are now editable at runtime, so `Board::roles()` and `Board::projects()` return snapshots.
- Settings changes are not recorded in the card event log.
- Changing the file by hand while the board runs is not picked up until restart (the UI is the live path).

## Alternatives rejected
- **Settings in the board database:** one store, but a migration for existing files and secrets inside the card store.
- **Settings as MCP tools:** an agent could grant itself access.

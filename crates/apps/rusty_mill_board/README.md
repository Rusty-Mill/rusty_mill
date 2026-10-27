# rusty_mill_board

Mill Board: a kanban board that humans and agents share, over MCP and a web UI.
The board is the only state; it enforces who may move a card where.

```text
RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788                          # MCP over stdio
RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788 --http 127.0.0.1:8787 --auth-token S
rusty_mill_board --http 127.0.0.1:8787                         # in memory, gone when it stops
# UI: http://127.0.0.1:8787/#token=S   API: POST /api/<op>   MCP: /mcp
```

`--issuer https://board.example.com` adds OAuth sign-in for `/mcp`, so a claude.ai connector can
"Sign in now" with one of the board's own tokens; see
[deploy/README.md](deploy/README.md#signing-in-from-claudeai-oauth).

- **Columns:** backlog, ready, building, qa, review, done, blocked.
- **Rules:** each move needs a capability (triage, build, qa, review, merge); leaving
  building/qa/review needs your claim. A role is a set of capabilities; roles are
  defined by the caller, `Roles::default()` is a starting set. `actor`/`role` are
  trusted as given: this is a workflow guard between cooperating agents, and the
  bearer token is the authentication.
- **Order:** `card_take` looks in Review, QA, Building, then Ready; within one, the highest `priority` (0, 1, 3, 5)
  first, then the oldest. A card waits until every card in its `deps` is Done (a dependency no longer on the board
  holds nothing up); naming it explicitly says why. `card_plan` sets both and needs Triage. The tags `agent`,
  `board:…` and `claim:…` are the board's own ([ADR-0012](../../../docs/adr/0012-one-task-store.md)): a card cannot
  carry them, and the GitHub sync drops such labels when it imports an issue.
- **Projects:** one board serves several applications of a monorepo, and several repositories.
  `--projects FILE` names the projects and the repository paths each may change
  (`{"rocket_league": ["crates/apps/rocket_league"]}`, or `{"repos": {"rusty_mill": {...}, "other": {...}}}`
  for several repositories; project names are unique across them). Every repository also has a project named
  like itself that owns the paths no other project of it does (a flat file is the repository `default`). A card belongs to one project (old cards to
  `default`; `card_add` takes `project`, `card_assign` moves one and needs Triage). A credential in
  `--identities` may carry `"projects": [...]`: it then cannot see, take or change any other project's cards,
  checked inside every write's compare-and-swap, so a card re-assigned mid-call is still refused.
  `board_list` and `card_take` take a `project` filter. The UI has a two-pane side panel: repositories with card counts, and for the chosen one
  its projects; both filter the board, and new cards go to the chosen project (else the repository's own). An
  Agents list under the repositories (everyone holding a claim, with their count) filters by claimant. Each agent
  has its own colour (twelve hues; two agents never share one), shown in the list and on every card it holds:
  a coloured edge, a tint and a chip, whose dot pulses while the card is in building, qa or review. The runner
  enforces the paths too: see its README. GitHub sync is project-blind (imported issues land in `default`);
  `board_events` carries no card text but does not hide other projects' event metadata.
- **Tools:** `board_whoami`, `board_list`, `board_roles`, `board_projects`, `board_owns`, `board_events`, `card_take`,
  `card_get`, `card_add`, `card_assign`, `card_move`, `card_claim`, `card_release`, `card_update`, `card_plan`, `card_comment`. The web API
  exposes the same table, so the two cannot drift.
- **Settings.** The ⚙ button opens a view to add and remove agents (credentials: name, role, project limit; a
  token is shown once; "New token" replaces it), edit repositories and their projects, edit roles, and configure
  the GitHub sync. It needs the `admin` capability (the default `human` role has it). Run with
  `--settings-dir DIR`: it keeps `identities.json`, `projects.json`, `roles.json` and `github.json` there
  (secrets owner-only), changes take effect at once, and a first start creates an `admin` credential and
  prints its token once. Without files the view is read-only. A change that would lock everyone out, or drop a
  project that still has cards, is refused. Not available over MCP. ADR-0004.
- **Models connect themselves.** Run one HTTP server with `--identities FILE` (a token per model, bound to an
  actor and role) and register `/mcp` in each model's own client; a model then pulls work with `card_take`,
  comments, and moves it on, using its own subscription session. See
  [`docs/connecting-models.md`](docs/connecting-models.md) for setup and the paste-in working loop.
- **As a service:** `GET /health` (200, or 503 while rusty_tick is unreachable); SIGTERM stops accepting, waits
  for running requests, then exits. A systemd unit, an env example and the install and rollback steps are in
  [`deploy/`](deploy/README.md). ADR-0005.
- **Event ids** are UUIDv7 assigned inside the commit, so id order is commit order and doubles as the polling
  cursor (`board_events` returns a `cursor`; `tail: true` starts a watcher at the current end).
- **Storage: rusty_tick** (`--tick-url`, token only from the `RUSTY_TICK_TOKEN` environment variable; [ADR-0012](../../../docs/adr/0012-one-task-store.md)):
  the board is the rusty_tick tasks tagged `agent`, so the owner reads and edits the same tasks on the phone or web.
  Column and claim are `board:`/`claim:` tags, the project is the task's list (the Inbox is `default`), comments are
  rusty_tick comments with their author, and dependencies, the forge link and the execution link are one `board`
  document per task. Every write is one request fenced on the task version the board read, and every write moves
  it (rusty_tick moves it for a board document or comment written with `If-Task-Match`), so the version names the
  whole card. A comment is an explicit append, so one the owner adds meanwhile is kept. Only `card_plan` with both a
  priority and dependencies takes two requests; if the second fails it reports an error, and the same plan again
  completes it.
  Tags come back lowercase, so actor names must be lowercase. Events are kept in memory only: the board's own
  writes plus an `edited` event (actor `tick`) when a task changes in rusty_tick. `/health` is 503 while rusty_tick
  is down. rusty_tick keeps the data, so back up its data directory. Without `--tick-url` the board lives in
  memory (tests, demos).
  [`tools/migrate_artifact_board.py`](tools/migrate_artifact_board.py) moves the Mill Board artifact's cards
  into rusty_tick once (dry run by default, safe to re-run, `--rollback --yes` trashes what it wrote). The
  board's own `--store` file and `--backup-dir` were removed once that migration had run.

- **GitHub sync** (`--github-repo owner/name`, token in `GITHUB_TOKEN`, every
  `--github-secs`, default 60): stateless reconcile between cards and issues.
  The board owns state: the column is one `board:<column>` label and a Done card
  closes its issue; other labels are kept. Title and body are merged three ways against the text both sides last agreed on (a hash on the card),
  not by timestamps: one side changed it, that side wins; both did, the board wins and it is reported. An issue
  made from a card carries a hidden `<!-- mill-board:card=… -->` marker, so a run interrupted before the link
  was saved adopts that issue instead of creating a duplicate; the marker also records the text the issue was created
  with, so an edit made on GitHub in that window is merged, not overwritten. Only the marker is stripped from a pulled
  body. A pull is applied only if the card is unchanged since the run read it; otherwise it waits for the next run.
  Unlinked open issues become Backlog cards (closed ones are not imported). An imported card's id is derived from
  the issue number, so an import interrupted before its link is relinked, not repeated or exported as a new issue;
  the id ignores the repository, so give each repository its own board. An issue
  closed on GitHub while its card is not Done is logged as drift, not reopened or
  moved. The sync actor holds no capabilities, so it cannot move cards. Needs
  issues read/write on the repo; `--github-api` points at GitHub Enterprise Server.

UI: the card dialog is kept in step with the board without touching unsaved text; a change to the text under an
unsaved edit raises a conflict (keep mine / take theirs), and `card_update` takes the `rev` you last saw and refuses a
stale one. `tests/ui/drafts.mjs` checks this in a real browser (Playwright; see the file's header).

Layout: `domain` (pure) / `store` + `tick` (port and adapters) / `board` (service) /
`api` (operation table) / `mcp` + `web` (surfaces) / `sync` (forge port + reconcile) +
`github` (adapter).

Optional runner: [`runner/`](runner/README.md) (`mill-runner`) works cards through stages by running
a sandboxed command per column; the board does not depend on it. A BBP conductor is designed but
paused (ADR-0001).

Export validated project ownership without opening a store or services:
`rusty_mill_board --projects projects.json --export-projects`. The JSON uses the
existing projects file format and can feed
[Orch change-impact reports](../rusty_orch/docs/change-impact.md).

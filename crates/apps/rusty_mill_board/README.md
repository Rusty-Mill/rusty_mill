# rusty_mill_board

Mill Board: a kanban board that humans and agents share, over MCP and a web UI.
The board is the only state; it enforces who may move a card where.

```text
RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788                          # MCP over stdio
RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788 --http 127.0.0.1:8787 --auth-token S
rusty_mill_board --http 127.0.0.1:8787                         # in memory, gone when it stops
# UI: http://127.0.0.1:8787 (asks for the token; #token=S also works)   API: POST /api/<op>   MCP: /mcp
```

`--issuer https://board.example.com` adds OAuth sign-in for `/mcp`, so a claude.ai connector can
"Sign in now" with one of the board's own tokens; see
[deploy/README.md](deploy/README.md#signing-in-from-claudeai-oauth).

- **Columns:** set by an admin (Settings > Columns, kept in `workflow.json`); `board_columns` lists them and
  the moves between them. The default is backlog, ready, building, qa, review, done, blocked. Each column has a
  kind: `queue` (waits to be taken; `card_take` enters it into the first work column it moves to), `work`
  (claimed; leaving needs the claim), or one of the fixed `backlog`, `done` and `blocked`, which can be relabelled
  and reordered but not removed. A column id is what cards store (`board:<id>` in rusty_tick) and never changes;
  its label is what people see. A column that still holds cards cannot be removed, and a card whose column the
  workflow lacks reads as backlog. A move may be a **gate** (a human approval): it needs its capability and
  `admin`, even with movement rules off.
- **Factory workflow:** [`deploy/factory.workflow.json`](deploy/factory.workflow.json) is the software factory's
  stages: Backlog, Ready, Planning, Architecting, Building, V&Ving, Done, Blocked. Its gates (Backlog to Ready,
  Architecting to Building, V&Ving to Done by `merge`, and every move out of Blocked, so blocking is no way around
  an approval) need `admin`; `close` may still record V&Ving to Done for a PR already merged on the forge. With
  movement rules off, a move that jumps over a gate still needs `admin`, and V&Ving to Done still needs `close`
  or the gated `merge`. Each work column names its `takers`, so `card_take` gives a planner
  Ready and Planning, an architect Architecting, a tester Planning to V&Ving, a builder Building, and a reviewer
  every work column. A move may list more than one capability (one transition each); holding any one is enough.
  Install it by copying it to `workflow.json`, after re-filing the cards of columns it drops (below).
- **Re-filing cards:** `rusty_mill_board --tick-url URL --retag-columns qa=v_and_ving,review=v_and_ving` prints
  the plan as JSON (each card's id, revision, from and to). `--apply --retag-record FILE` (board stopped) creates
  FILE exclusively and syncs the plan into it, then, for each card still at its planned revision, records `begin`,
  retags it (keeping its claim) and records `done`, each line synced. `--undo-retag FILE` puts back each card
  recorded `done` that is in the new column at the very next revision; a card changed since, never reached, or
  whose write's outcome is unknown is left alone and reported. Unix only: elsewhere the record's directory entry
  cannot be synced, so `--apply` refuses.
- **Rules:** each move needs the capability the workflow gives it (by default triage, build, test, review, merge,
  close; `plan` and `architect` serve the factory workflow; `test` was `qa`, which stored roles may still say); any open card may be moved to blocked; leaving a work column needs your claim. `close` moves a card from building or qa straight to done when
  its work already merged on the forge, and lets `card_add` with `done: true` record merged work that never
  had a card (bookkeeping, not a review: it never makes a card someone's work in `card_take`). The default
  `builder` role holds it, and `project`: `project_add` adds a project (or a repository) whose paths overlap no
  other project's. It never changes or removes one, so no credential's reach widens; it needs the board's
  projects file (`--projects` or `--settings-dir`). A role is a set of capabilities; roles are
  defined by the caller, `Roles::default()` is a starting set. `actor`/`role` are
  trusted as given: this is a workflow guard between cooperating agents, and the
  bearer token is the authentication.
- **Order:** `card_take` looks in the queue and work columns nearest Done first (by default Review, QA, Building,
  then Ready); within one, the highest `priority` (0, 1, 3, 5)
  first, then the oldest. A card waits until every card in its `deps` is Done (a dependency no longer on the board
  holds nothing up); naming it explicitly says why. It never takes a card with a `superseded_by` link (newer cards
  that replace it): close that card, or re-scope it and clear the link. `related` links are informational.
  `card_plan` sets priority, all three link lists, `parent`, `milestone` and a target date (`target_ms`, `null` clears it), and needs Triage. The tags `agent`,
  `board:…`, `claim:…`, `number:…`, `archived:…` and `restored:…` are the board's own ([ADR-0012](../../../docs/adr/0012-one-task-store.md)): a card cannot
  carry them, and the GitHub sync drops such labels when it imports an issue.
- **Projects:** one board serves several applications of a monorepo, and several repositories.
  `--projects FILE` names the projects and the repository paths each may change
  (`{"rocket_league": ["crates/apps/rocket_league"]}`, or `{"repos": {"rusty_mill": {...}, "other": {...}}}`
  for several repositories; project names are unique across them). Every repository also has a project named
  like itself that owns the paths no other project of it does (a flat file is the repository `default`). A card belongs to one project (one that names none, and old cards, to
  `default`, shown in the web UI as the Inbox, the first project chip; `card_add` takes `project`, `card_assign` moves one and needs Triage, or `project` for a card nobody else has claimed; a Done card stays Done and needs Triage or `close`). A credential in
  `--identities` may carry `"projects": [...]`: it then cannot see, take or change any other project's cards,
  checked inside every write's compare-and-swap, so a card re-assigned mid-call is still refused.
  `board_list` and `card_take` take a `project` filter. The UI's scope bar has a repository picker and then every project
  as a chip (a repository's only project is the repository itself), with card counts; both filter the board, and new cards go to the chosen project (else the repository's own). The
  Agents chips after them (everyone holding a claim, with their count) filter by claimant. Each agent
  has its own colour (twelve hues; two agents never share one), shown on its chip and on every card it holds,
  as a dot that pulses while the card is in a work column. The runner
  enforces the paths too: see its README. GitHub sync is project-blind (imported issues land in `default`);
  `board_events` carries no card text but does not hide other projects' event metadata.
- **Tools:** `board_whoami`, `board_list`, `board_roles`, `board_projects`, `board_owns`, `board_events`, `card_take`,
  `card_get`, `card_trace`, `card_add`, `project_add`, `card_assign`, `card_move`, `card_claim`, `card_release`, `card_close`,
  `card_archive`, `card_restore`, `card_archive_done`, `card_update`, `card_plan`, `card_comment`, `board_milestones`, `milestone_add`,
  `milestone_update`, `inbox_list`, `inbox_act`, `board_number_cards`. The web API
  exposes the same table, so the two cannot drift.
- **Milestones.** A project's dated goals (`src/milestones.rs`): a title and a target date, closed when reached, never
  deleted. `milestone_add` and `milestone_update` need Triage and the project in scope; `board_milestones` lists what the
  caller may see. In rusty_tick a milestone is a task tagged `milestone` (never `agent`, so never a card) in the
  project's list, its `dueMs` the target; without `--tick-url` they live in memory.
- **Traceability** (`src/trace.rs`). A card may name one `parent`, the card it was split from (a strict tree: a loop
  or a chain deeper than 32 is refused), and one `milestone` of its own project; the two are separate, so a card can
  have both. `card_add` takes `parent` without Triage (whoever splits work records it); `card_plan` sets or clears
  either. Both live in the card's board document in rusty_tick beside `deps`, so tick's schema is unchanged and its
  own UI does not show them. Progress is derived, never stored: `card_trace` gives a card's chain up to its root and
  the tree below it, each node with a rollup of its direct children (total, done, blocked); `board_milestones` gives
  each milestone the same rollup of its cards. When all are Done, the parent or milestone is flagged
  `ready_to_close`; it never closes itself. The UI shows the chain and the children in the card dialog, a pieces
  count on a parent's ticket, parent and milestone fields in the dialog, and progress beside each milestone on the
  roadmap (`tests/ui/trace.mjs`).
- **Archive.** A Done card leaves the board's default views (`board_list`, the Done column) once it is archived, or
  14 days after it was done or last restored (`board_list`'s `done_days`; 0 keeps every unarchived Done card). It is
  still Done: `card_get`, the roadmap, metrics and dependencies see it as before. `card_restore` puts any such card
  back, starting its 14 days again without changing its ship time. `card_archive` and `card_restore` need `close`;
  `card_archive_done` archives every Done card at once (optionally one project's, done or restored at least
  `older_than_days` ago), checks each card again as it commits, and needs `triage`. `board_list` with
  `archived: "only"` lists the archive. The state is an `archived:<ms>` or `restored:<ms>` tag on the rusty_tick
  task, beside `number:<n>`, so there is no migration.
- **Project status.** A project is `active` (the default), `paused`, `experiment` or `archived`, kept in
  `project-status.json` in the settings directory (only non-active projects are listed; the projects file keeps its
  format). Settings > Repositories sets it per project (`settings_project_status_set`, admin), written before it
  applies. Paused and archived projects refuse `card_add`, forge imports (retried once active), open cards moved in
  with `card_assign` (a Done card may still be re-filed there), `card_claim`, and moves into a work column from
  outside one; `card_take` hands out none of their work. Work already under way may move on and finish. An
  experiment takes cards and hands out work. `board_list` lists projects that take work (active and experiment) by
  default: `project_status: [...]` lists others, and naming a `project` lists it whatever its status.
  `board_projects` reports `status` for non-active projects. The board's status picker (Active, Experiment, Paused,
  Archived, All projects) starts on Active. A paused or archived project must be made active before it is removed
  or renamed, so no status outlives its project.
- **Shipped needs evidence.** A Done card counts as shipped only with evidence: a merged PR or any http(s) link
  with a host that shows the work (printable ASCII, at most 512 bytes; `card_evidence`, or `evidence` on
  `card_add`, both needing `close`, `merge` or `triage`), at most 8 per card, kept in the card's rusty_tick board
  document. A dropped card takes none. Without evidence it is a prototype: `board_metrics` counts it as
  `prototype` (per week and per project) and leaves it out of shipped and lead time; the Metrics view shows
  prototypes beside shipped work. Cards Done before evidence existed are backfilled once, with the board stopped:
  `rusty_mill_board --tick-url ... --settings-dir ... --backfill-evidence rusty_mill=Rusty-Mill/rusty_mill`
  prints which Done cards (not dropped, no evidence yet) would get the pull requests their body and comments
  name (`https://github.com/O/N/pull/N` anywhere; `PR #N` or `pull request #N` in a mapped repository's
  projects; never a bare `#N`), at most 8 each; add `--backfill-record FILE` to write, and
  `--undo-backfill FILE` clears what it wrote. Observed external refs (ADR-0006) never count. Done cards that name no pull request read as prototypes
  until someone adds a link. The plan lists, per link, every place it is named (the text around it): read it, since a card
  may say "superseded by PR #5". Add `--check-merged` (with `GITHUB_TOKEN`, read access to pull requests) to ask GitHub
  whether each one merged and keep only those; dropped links go to stderr, and a failed lookup (auth, rate limit)
  stops the run before anything is written. A card changed while the backfill runs is
  skipped and reported.
  Each run writes its own `--backfill-record` file (an existing one is refused); undo a run with that run's file.
- **Close reasons (dropped work).** `card_close` closes an unstarted card (Backlog or a queue) without shipping
  it, for a required `reason`: `wont_do`, `duplicate` or `superseded`; an optional `note` becomes the closer's
  comment. It needs `triage`, and is refused for started, blocked or Done cards. The card goes to Done with no
  `done_ms` and a `closed: {reason, at_ms}`, so it never counts as shipped; `board_metrics` reports it as dropped,
  per week (`weeks[].dropped`), per project (`by_project[].dropped`) and per reason (`dropped`, last 30 days).
  In rusty_tick the task is closed as `wontdo` with a `closed:<reason>:<ms>` tag, in one write, so summaries see the
  reason without reading documents; a task marked won't-do in rusty_tick itself reads as `wont_do`.
  Rollups (`card_trace`, `board_milestones`) count dropped children apart from done ones, and a parent or
  milestone is `ready_to_close` only once every child is Done and at least one shipped.
- **Prompt (#745).** A card's `prompt` holds instructions for whoever takes it, apart from the body: `card_update`
  sets it (`""` clears it, at most 32 KiB), `card_get`/`board_list` return it, and the card dialog shows it in its own
  box with a Copy prompt button. It lives in the card's rusty_tick `board` document, not on the task, so rusty_tick's
  own UI doesn't show it and nothing migrates. A prompt edited with the title, body or tags is two writes to rusty_tick;
  if only the first lands, the answer is `409` with `saved_rev` (what was saved, and the revision to finish on), and
  the dialog keeps the unsaved prompt and says so.
- **By date (#744).** `board_list` takes `created_from`/`created_to`, `updated_from`/`updated_to` and
  `moved_from`/`moved_to` (Unix ms, `from` inclusive, `to` exclusive). Moves come from rusty_tick's durable move
  history (`GET /moves`), so they survive restarts; entering the board counts as a move.
- **Summaries and pages (#1429, S7).** `board_list` returns summaries by default: each card's task fields only (no
  body, prompt, comments or links), read from rusty_tick's tasks alone, about a third of a full read. Pass
  `view: "full"` for whole cards, or `card_get` one. `limit` (1–500) pages either view in list order,
  `(created_ms, id)`, and returns `next_cursor` until the listing is done; a page also stops at 512 KiB. A cursor is
  opaque, tied to its query, and survives a restart; scope is applied to every page. Without `limit`, every matching
  card is returned.
- **Decision inbox (#595).** `inbox_list` and `inbox_act` (admin only, and only through a token or OAuth credential, never a caller-named actor) show and decide the open BBP escalations
  of the tasks linked from cards. Mill Board stores none of it: run with `--bbp-bin PATH --bbp-root DIR` on the
  orch host and it runs `bbp inbox` and fenced `bbp human` commands, so the host stays the only writer of the log.
- **Settings.** The ⚙ button opens a view to add and remove agents (credentials: name, role, project limit; a
  token is shown once; "New token" replaces it), edit repositories and their projects, edit roles, and configure
  the GitHub sync. It needs the `admin` capability (the default `human` role has it). Run with
  `--settings-dir DIR`: it keeps `identities.json`, `projects.json`, `roles.json`, `github.json`, `rules.json` and `workflow.json` there
  (secrets owner-only), changes take effect at once, and a first start creates an `admin` credential and
  prints its token once. Without files the view is read-only. A change that would lock everyone out, or drop a
  project that still has cards, is refused. Roles explain each capability. **Apps** lists the apps signed in
  through OAuth (with `--issuer`) and revokes one (it can sign in again); **Columns** adds, relabels, reorders and removes columns and edits the moves between them;
  **Rules** turns card movement
  rules off and on (off: any move but out of Done, by anyone holding a capability, each logged; on by
  default); **Server** shows, read-only, where
  the cards live, the issuer, the accepted hosts and the settings files. Not available over MCP. ADR-0004.
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
  rusty_tick comments with their author, and dependencies and other links, the forge link and the execution link are one `board`
  document per task. Every write is one request fenced on the task version the board read, and every write moves
  it (rusty_tick moves it for a board document or comment written with `If-Task-Match`), so the version names the
  whole card. A comment is an explicit append, so one the owner adds meanwhile is kept. Only `card_plan` with both a
  priority and links takes two requests; if the second fails it reports an error, and the same plan again
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
Cards show their priority (High 5, Medium 3, Low 1; none shows nothing), sort highest first in each column, and
the dialog sets it through `card_plan` (`tests/ui/priority.mjs`).
The board reads like a press room's job board ([PRODUCT.md](PRODUCT.md); direction in `.impeccable/surfaces/board.md`):
a colour strip across the top shows each column's open cards and jumps to it; empty columns fold to a spine; process
inks carry state (cyan in work, yellow queued, vermilion blocked) and magenta is kept for what waits on you, a card at a
gate (a work column whose next move is a human approval, with an Approve button) or with a decision open. The
typefaces (Hanken Grotesk, Barlow Condensed; SIL OFL) are served from the binary at `/ui/fonts/`, so the page fetches
nothing from elsewhere.

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

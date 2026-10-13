# rusty_mill_board

Mill Board: a kanban board that humans and agents share, over MCP and a web UI.
The board is the only state; it enforces who may move a card where.

```text
rusty_mill_board --store board.mmap                                  # MCP over stdio
rusty_mill_board --store board.mmap --http 127.0.0.1:8787 --auth-token S
# UI: http://127.0.0.1:8787/#token=S   API: POST /api/<op>   MCP: /mcp
```

- **Columns:** backlog, ready, building, qa, review, done, blocked.
- **Rules:** each move needs a capability (triage, build, qa, review, merge); leaving
  building/qa/review needs your claim. A role is a set of capabilities; roles are
  defined by the caller, `Roles::default()` is a starting set. `actor`/`role` are
  trusted as given: this is a workflow guard between cooperating agents, and the
  bearer token is the authentication.
- **Tools:** `board_list`, `board_roles`, `board_events`, `card_get`, `card_add`,
  `card_move`, `card_claim`, `card_release`, `card_update`, `card_comment`. The web API
  exposes the same table, so the two cannot drift.
- **Storage:** `rusty_multimodal_db_engine` (cards and an append-only event log in one
  table; claims are atomic via `replace_if`), or in memory when `--store` is omitted.
  The event id is a UUIDv7 and doubles as the polling cursor (`board_events after=`).

- **GitHub sync** (`--github-repo owner/name`, token in `GITHUB_TOKEN`, every
  `--github-secs`, default 60): stateless reconcile between cards and issues.
  The board owns state: the column is one `board:<column>` label and a Done card
  closes its issue; other labels are kept. Title and body are last-write-wins.
  Unlinked open issues become Backlog cards (closed ones are not imported). An issue
  closed on GitHub while its card is not Done is logged as drift, not reopened or
  moved. The sync actor holds no capabilities, so it cannot move cards. Needs
  issues read/write on the repo; `--github-api` points at GitHub Enterprise Server.

Layout: `domain` (pure) / `store` + `mmdb` (port and adapters) / `board` (service) /
`api` (operation table) / `mcp` + `web` (surfaces) / `sync` (forge port + reconcile) +
`github` (adapter).

Optional runner: [`runner/`](runner/README.md) (`mill-runner`) works cards through stages by running
a sandboxed command per column; the board does not depend on it. A BBP conductor is designed but
paused (ADR-0001).

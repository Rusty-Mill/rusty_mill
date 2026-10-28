# Connecting models to the board

The board is an MCP server. A model that already runs in its own logged-in session (Claude Code, Codex,
Gemini CLI, a desktop app) connects as an MCP client, pulls cards, does the work with its own tools and
subscription, and reports back through the board. No model API key is involved on the board's side.

## 1. One shared server, a token per model

Stdio would start a separate server per client, and they would fight over the store. Run one HTTP server:

```sh
# a token each (16+ chars), bound to an actor name and a role
cat > ~/mill-identities.json <<'JSON'
[{"token": "REPLACE-claude-token-xxxxxxxx", "actor": "claude-code", "role": "builder"},
 {"token": "REPLACE-codex-token-xxxxxxxxx", "actor": "codex",       "role": "reviewer"},
 {"token": "REPLACE-gemini-token-xxxxxxxx", "actor": "gemini",      "role": "qa"},
 {"token": "REPLACE-human-token-xxxxxxxxxx", "actor": "nano",       "role": "human"}]
JSON
chmod 600 ~/mill-identities.json                 # refused otherwise
RUSTY_TICK_TOKEN=T rusty_mill_board --tick-url http://127.0.0.1:8788 --http 127.0.0.1:8787 --identities ~/mill-identities.json
```

To keep a model inside one application of a monorepo, add `"projects": ["rocket_league"]` to its entry (names
from the board's `--projects FILE`, which the server checks at start): that token cannot see, take or change
cards of any other project, and `card_take` only ever returns its own.

The simplest start is `rusty_mill_board --tick-url http://127.0.0.1:8788 --http 127.0.0.1:8787 --settings-dir ~/.mill`
(with `RUSTY_TICK_TOKEN` set):
the first start creates `~/.mill/identities.json` with an `admin` agent and prints its token once (open
`http://127.0.0.1:8787/#token=...`), and agents are then added from the ⚙ settings view, which shows each new
token once for you to paste into that model's client.

Generate tokens by hand with `openssl rand -hex 24`. A token fixes the caller's actor and role: a model cannot
act as another, and it does not pass `actor`/`role` at all. Roles (see `board_roles`): `human` (everything),
`builder` (Build), `qa` (Qa), `reviewer` (Review, Merge). The UI is at `http://127.0.0.1:8787/`; give the
human token in the page's address (`#token=...`).

## 2. Register the board in each model's client

Claude Code (verified with 2.1.296 against this board: two separate `claude -p` sessions, one as `builder`
and one as `qa`, passed a card between them over MCP):

```sh
claude mcp add --transport http mill http://127.0.0.1:8787/mcp --header "Authorization: Bearer <claude token>"
```

For a one-off session instead of a registered server, write the config to a file and run
`claude -p "<the working loop>" --mcp-config mill.json --strict-mcp-config --allowedTools mcp__mill__board_whoami mcp__mill__card_take mcp__mill__card_comment mcp__mill__card_move`
with `{"mcpServers":{"mill":{"type":"http","url":"http://127.0.0.1:8787/mcp","headers":{"Authorization":"Bearer <token>"}}}}`
in `mill.json` (chmod 600; it holds the token).

Codex CLI and Gemini CLI take an HTTP MCP server with a bearer header in their config files
(`~/.codex/config.toml`, `~/.gemini/settings.json`). The exact keys differ by version, so check each
tool's MCP docs; the URL is `http://127.0.0.1:8787/mcp` and the header is `Authorization: Bearer <token>`.
Over a network, put TLS in front (the server speaks plain HTTP) and set `--allowed-host`.

## 3. The working loop (paste into CLAUDE.md / AGENTS.md / GEMINI.md, or say it in the session)

```text
You work from the Mill Board (MCP server "mill"). Humans decide what is ready and what is merged.
1. Call board_whoami once to see your role.
2. Call card_take. It returns your next card, already claimed for you, or {card: null} when nothing
   is waiting for your role. If null, say so and stop; do not invent work.
3. Read the card and its comments (earlier notes may say why a previous attempt failed). Do the work in
   your own environment.
4. Call card_comment with what you did, what you verified, and anything the next person needs.
5. Call card_move to the next column: builder -> qa when the work is done; qa -> review if it passes, or
   back to building with the failure in a comment; reviewer -> done only when you would merge it.
   If the card's work already merged on the forge (its PR is merged), comment with the PR link and move
   it from building or qa straight to done; this needs the `close` capability.
6. If a move is refused, the message names the missing capability. Do not look for a way around it:
   comment on the card and leave it for a person.
Poll board_events (after = the last event id you saw) when you are asked to watch the board.
```

## Notes

- **A shared `--auth-token` authenticates but does not separate roles**: every holder can act as any role,
  including `human`. Use `--identities` for anything more than one trusted user. The server refuses to listen
  beyond loopback with no authentication at all.
- **Waking a model.** Models cannot be pushed to. Tell a session to check the board ("take the next card"),
  or loop it (`/loop 10m take the next card from the board`). `board_events` is the cheap way to see
  whether anything changed.
- **Claims are the lock.** `card_take` claims atomically; two models never get the same card. A person
  holding `Triage` can take a card back from a stuck model at any time.
- **Imported GitHub issues** become Backlog cards; a person moves them to Ready, which is the check before
  a model is handed their text as work.
- **`mill-runner`** (optional) calls the API with explicit roles per stage, so it needs `--auth-token`
  mode or a board of its own, not `--identities`.

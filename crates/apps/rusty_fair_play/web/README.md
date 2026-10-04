# Fair Play — web UI for `rusty_fair_play`

React 18 + TypeScript + Vite + Tailwind + Zustand + React Router (hash routes),
the same tooling as `crates/apps/rusty_tick/web`. Talks to the `rusty_fair_play`
server (data lives in `rusty_multimodal_db`'s `fair_play` domain, ADR-0137), or
runs alone against an in-browser demo backend.

![The deck with the detail pane](docs/screenshots/deck-detail.png)

## Run it

```sh
cargo build -p rusty_fair_play   # from the repo root; the binary the tests below start
npm ci
npm run dev                      # http://localhost:5173, proxies /api and /health to $FAIR_PLAY_BACKEND (default 127.0.0.1:8790)
npm test                         # unit/component tests (jsdom, in-memory backend) incl. the API contract against MemoryAdapter
npm run test:integration         # the same API contract against the real rusty_fair_play binary
npm run build                    # typecheck + production bundle into dist/
npm run e2e                      # Playwright flows against the real binary serving dist/ from a fresh .e2e-data; also takes the screenshots
```

Serve it for real with `target/debug/rusty_fair_play --data-dir ./data --web-dir web/dist`
and open `http://127.0.0.1:8790/`. The deck ships in the binary and is loaded on
first start, so the board is full straight away. The UI boots without asking for
anything; if the server was started with `RUSTY_FAIR_PLAY_TOKEN`, the first `401`
brings up the token prompt (kept in sessionStorage, or localStorage with
"remember on this device"). Demo mode: open `/?adapter=memory` — a dozen real
deck cards in the browser's localStorage, nothing sent anywhere.

## What it does

- `#/deck` — the board: six suit shelves in deck order with counts, a tile per
  card (suit stripe, number, name, owner chip, `edited`/`custom`/`split · N`
  badges). Filter by suit, owner, state, leaves only, and name; `/` focuses search.
  "New card…" makes a custom top-level card (`POST /cards`) and opens it.
- `#/deck/:cardId` — the detail pane (a drawer over the board on narrow screens):
  breadcrumb up the split chain, inline rename, "Deal to…", Conception /
  Planning / Execution, the minimum standard of care as an editable list, notes —
  one Save sends only the changed fields in one `PATCH`. Deck cards get a
  "Changes from the original" disclosure (field-by-field diff from `/baseline`)
  and a confirmed "Reset to original"; custom cards say so. Children are listed in
  position order with move up/down (`PUT /position`, swapping positions) and a
  "Split…" dialog (child rows with owners, and whom to hand the parent to).
- `#/players` — people with "holds N cards (M leaves)", inline rename, add (a
  `409` shows as "already exists").
- `#/balance` — per person, all-cards and leaf-only bars with a per-suit row, then
  "Still undealt": the unassigned leaf cards grouped by suit with a quick deal.

## How it fits together

- `src/api/` — one `ApiClient` interface, two adapters: `HttpAdapter` (`/api/v1`,
  same origin) and `MemoryAdapter`, which keeps the same rules in the browser
  (state derived against a kept baseline, splits with positions, reset, unknown
  owner and cycle refusals, 409 on a duplicate name). `contract.ts` is one
  behaviour suite run against both (`npm test` and `npm run test:integration`).
- `src/store/data.ts` — a Zustand store holding the snapshot; writes await the
  server and merge the returned card(s) in; the snapshot is refreshed on window
  focus and every 30 s. `derive.ts` computes children, leaves, chains, balance
  (all vs leaf-only) and counts from the cards array on the client.
- `src/features/` — deck (board, filters, detail pane, split dialog, baseline
  block), players, balance. `src/components/` — Dialog, Confirm, Toasts, Tooltip,
  the rail, badges. `src/styles/tokens.css` — every colour as a CSS variable,
  including the six suit colours, with a dark set under `[data-theme="dark"]`
  (the rail's theme button cycles system / light / dark).

| | |
|---|---|
| ![Balance](docs/screenshots/balance.png) | ![Players](docs/screenshots/players.png) |

## Known gaps

- Re-parenting a card (`parentCardId`) and changing a card's suit are supported by
  the API and the adapters but not exposed; reset is the only way a suit changes.
- Concurrent edits: the last write wins and the 30 s refresh shows it; there is no
  etag/412 handling because the API has none.
- Moving a child swaps two positions with two `PUT`s; a crash between them can
  leave two children on one position (the order then falls back to id).
- The demo deck is twelve cards, not the hundred the binary ships.

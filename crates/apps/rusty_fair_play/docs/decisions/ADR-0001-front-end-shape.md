# ADR-0001: The Fair Play front end — an app crate over a domain libs crate, `rusty_tick`'s HTTP shape, a React board

Status: Accepted (implemented on one branch)
Date: 2026-10-04

## Context

ADR-0137 built the Fair Play domain as the database layer and left "a
card-style front end" for later. A browser cannot speak the database's
binary wire protocol, and the embedded API (`generic::fair_play`) is
richer than the socket one (an owner index, trees, state counts), so the
front end wants an HTTP layer that holds the stacks in-process.

Three things in the workspace fix the shape:

- **`rusty_tick`** is the precedent for a web app on this engine: a JSON
  API on `rusty_http`/`rusty_json`/`rusty_url` (its ADR-0001: a sans-IO
  router, one thread per connection, no async runtime), a static-file
  server for a `web/` React/TypeScript/Vite/Tailwind/Zustand UI, vitest
  with an in-browser adapter sharing a contract with the real one, and
  Playwright against the real binary.
- **ADR-0003's layer rule**: an app crate may depend on an app crate only
  within its own family. The domain was built inside `rusty_multimodal_db`,
  an app crate; the owner wants the front end at `crates/apps/`, its own
  family.
- **The design directive**: small, efficient, terse, modular, coherent.

## Decision

1. **An app crate at `crates/apps/rusty_fair_play` over a domain libs
   crate.** The domain moved out of `rusty_multimodal_db` into
   `crates/libs/storage/rusty_fair_play_domain` (records, stacks, queries,
   the seed loader with the deck embedded, the crash writer and the
   domain's own tests), and `rusty_multimodal_db` re-exports it as
   `generic::fair_play` exactly as it re-exports the engine (ADR-0124), so
   its wire adapters, `fair_play_server` and socket suite are unchanged.
   The app depends on the domain crate and the engine (for the query
   traits and the directory lock) and on no other app crate, which is
   what the layer rule asks. Not a module of the database crate: an HTTP
   JSON API and a web bundle are an application, not a database feature.
2. **`rusty_tick`'s shape, with the transport shared.** `api` (pure
   router), `dto` (wire shapes), `service` (rules, holding the three
   stacks). `server` and `static_files` were first copied from
   `rusty_tick` with its backend replaced by one `Api`; with two real
   call sites they became `crates/libs/net/rusty_serve` (a `Handler`
   trait, `Server<H>`, the bounds, the static file rules), and each
   app's `server.rs` is a few lines binding its router to it.
3. **One boot read.** `GET /snapshot` returns people and every card with
   its derived state; the UI computes children, leaves, chains, balance
   and counts client-side from a few hundred rows. Writes are per card
   (`PATCH`, `POST …/split`, `…/reset`, `PUT …/position`) and return the
   card. Every card carries an `etag` (the first eight bytes of
   SHA-256 over the stored record, so it costs nothing to keep) and a
   write may send it as `If-Match`; a mismatch is 412 with the current
   card, so a UI can reload rather than overwrite. A household's edits
   rarely race, so the header is optional and a bare write still wins.
4. **The deck ships in the binary.** The seed loader moved from
   `examples/support/` into `generic::fair_play::seed`, parsing text with
   file wrappers beside it, and the supplied CSV is `include_str!`ed as
   `DECK_CSV`; the server loads it on first start and `POST /seed` is
   idempotent. People and splits still come from CSV through the `seed`
   subcommand.
5. **Auth is optional.** `RUSTY_FAIR_PLAY_TOKEN` (16+ characters), when
   set, is a bearer token on `/api`; without it the binary serves
   loopback only and refuses `--allow-remote`. `rusty_tick` always
   requires a token; a family app on a laptop should not.
6. **The UI**: a deck board in six suit shelves (owner chip,
   `edited`/`custom`/split badges, filters, search), a card pane
   (deal, CPE, standards, notes, baseline diff and reset, ordered
   children, split dialog), a players page, and a balance page with
   all-cards and leaf-only bars and the "still undealt" leaves — the
   views ADR-0137's queries exist for. A `MemoryAdapter` runs the same
   rules in the browser for unit tests and a demo mode; the shared
   contract runs against it and the real binary.
7. **Deletes that keep the store well-formed.** ADR-0137 left the
   domain without a delete because a raw one orphans children or
   cards' owners. The domain now has the guarded ones: `delete_card`
   (leaves only), `unsplit_card` (the subtree, deepest first, parent
   kept: ADR-0137's merge/unsplit hook), `delete_person` (holding
   nothing), and `reorder_children` (one call, exact set, positions
   `0..n`) so a move is never two half-done writes. Each is a route.

## Consequences

- The web stack is the workspace's existing one; nothing new to learn or
  secure. Two CI jobs mirror `rusty_tick`'s.
- Deletes exist but refuse to orphan: a parent is unsplit first, a
  holder's cards reassigned first. A bare write still wins; `If-Match`
  is the client's choice, and the web UI always sends it.
- One server crate, `rusty_serve`; a third app gets it for free.
- The domain's home is now a libs crate; a future domain that two apps
  share has this shape to copy.

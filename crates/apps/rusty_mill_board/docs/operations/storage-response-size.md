# Storage response size incident (2026-11-09)

## Cause and evidence

The production MCP `board_whoami` succeeded; `card_get #959` returned
`storage error: rusty_tick: invalid http response: http body exceeded the maximum allowed size`.
No production records were written during diagnosis.

The request path is MCP (`mcp.rs`) -> board/application (`board.rs`) ->
`TickStore` (`tick.rs`) -> `TickHttp` -> blocking `Http` (`http.rs`) ->
`rusty_request::attempt` (`crates/libs/net/rusty_request/src/client.rs`) ->
`AsyncTransport::read_body_with_limit` (`rusty_http/src/async_tokio.rs`).
`rusty_http/src/body.rs::DEFAULT_MAX_BODY_LEN` is **1,048,576 bytes**;
`Error::BodyTooLarge` in `rusty_http/src/error.rs` supplies the final message.
`rusty_request::Error::InvalidResponse` and `TickHttp` add the outer context.
The board used the default, with no deployed response-size override.

Production loopback measurements, using the running board's storage identity:

| Storage GET | Records | Response bytes | Latency |
|---|---:|---:|---:|
| `/tags/agent/tasks` | 980 | **1,049,100** | 31 ms |
| `/docs/board` | 263 | 304,971 | 9 ms |
| `/docs/comment` | 1,100 | 840,865 | 30 ms |
| `/docs/board_number` | 980 | 189,042 | 4 ms |
| `/tags/milestone/tasks` | 36 | 18,486 | 4 ms |

The task collection exceeded the client limit by **524 bytes**. Responses were
valid JSON with identity encoding. The client limit is applied to HTTP entity
bytes after transfer framing, without content decompression in this path.
For Content-Length it rejects an oversized length before reading the body;
chunked and close-delimited bodies are also capped. It is not an MCP output cap.

Numbered card resolution calls `Store::list`, so even a request for one number
fetched every task, board document and comment. Roadmap, milestone progress,
trace and event observation also read task collections. Project/column filters
run after retrieval. A UUID get fetched a single task but *all* board documents
and comments. All these collection responses grew with unrelated work.

rusty_tick serves records from its authenticated user's engine-backed tables,
not a serialized database/journal download. Its tag index gives task IDs;
documents have a kind index. Tables use `.mmap` files and engine durability
files; tasks also have `tasks.journal`. The deployed units are
`/etc/systemd/system/{mill-board,rusty-tick}.service`, using
`/usr/local/bin/{rusty_mill_board,rusty_tick}`. The default installation's data
root is `~/.rusty_tick/data`, with multi-user data under `users/default` on this
host. Board settings live separately under `~/.mill-board/settings`.
No evidence of malformed JSON or a corrupt record caused this incident.

## Fix and compatibility

Add opt-in pages at the storage collection boundary, for tagged tasks and
all document kinds, including number reservations. No database migration,
record rewrite, dependencies or HTTP limit increase is required.

- `?limit=1..1000&after=<UUID>` returns the usual `tasks` or `docs` array plus
  `next` (last emitted UUID, or null). Short pages can have a continuation.
- UUID ordering is deterministic and unaffected by editing timestamps. The
  server gathers indexed IDs, then loads records one at a time, retaining one
  page of payloads instead of all matching bodies.
- Pages target 768 KiB of serialized records; one larger record can occupy a
  page by itself up to the unchanged 1 MiB transport limit minus 128 bytes of
  envelope headroom. An individual record beyond that returns an explicit error,
  never a partial record or silent truncation. The existing 64 KiB document
  write limit is unchanged; large prompts remain whole.
- `taskId=<UUID>` filters comment pages before the page budget. It scans the
  kind's IDs/bodies without retaining unrelated comments; no new persistent
  index or schema is introduced.
- Mill Board consumes *all* pages internally and refuses bad/non-advancing
  cursors and empty continuing pages. MCP schemas/results remain unchanged.
- UUID gets use `GET /docs/board/{id}` and task-filtered comments, keeping the
  existing task-version recheck and bounded retry for concurrent writes.
- Legacy clients omitting `limit` retain their original response shapes/order.
  Old tick servers ignore query parameters: upgrade tick **before** board.
- Pages are live views, not snapshot isolation across requests. An insertion
  behind a cursor can appear on the next complete read; edits cannot duplicate
  keys, and removed records disappear. This retains the adapter's documented
  weak list consistency and write version fences.

The complete Store/MCP list contract still needs O(total returned data) memory.
The wire/read buffer is bounded; this change does not claim constant-memory
whole-board responses. ID selection is O(matching IDs). Project/column filtering
still occurs in the application; numbered lookup reads only the `number:<n>` tag and the matching card (#1421).
Lists of project names use the existing small `/lists` read. Legacy unpaged
storage clients still need to opt into paging as their own collections grow.

## Regression and measurements

`tests/large_tick.rs` runs the Tick API test fixture over a real loopback
socket, using the default production HTTP client. The production tick router
and engine are tested separately in `rusty_tick/tests/api.rs`, respecting
ADR-0003's prohibition on Cargo dependencies between application families.
The HTTP fixture contains 240 cards in two projects, 4,128,000 prompt bytes, large notes/comments, parent/dependency links,
a milestone, concurrent fenced updates and collection reads. It checks all
cards/prompts, numbered lookup, roadmap, trace, milestone progress and events,
then restarts the HTTP fixture and compares the complete card collection for
equality. The engine-backed API regression separately reopens its temporary
store and compares large task/document collections for equality.
Unpaged task, board-document and comment reads reproduce the original error.
Before the adapter fix, a real tick-server version of this fixture failed
the single-card assertion with the same error. The committed loopback fixture
also exercises the unchanged HTTP limit without a cross-application dependency.

One debug-build HTTP-fixture run on this host (not production latency or a
performance guarantee):

| Measurement | Original | Fixed |
|---|---|---|
| UUID card read | BodyTooLarge | 57 ms, complete prompt/comment |
| Complete board read | BodyTooLarge | 517 ms, all 240 cards |
| Unpaged oversized fixture request | rejected | still rejected (limit preserved) |
| Process peak RSS | not measured for failed baseline | 59,760 KiB |

First-page response sizes were 780,303 bytes for tasks, 784,602 for board
documents and 784,360 for comments (all below the unchanged 1 MiB cap).

RSS includes the in-memory fixture, loopback HTTP server, and several retained
whole-board copies;
it is not a per-request allocation measurement. API tests assert the serialized
multi-record page bound, unchanged legacy shapes, malformed-cursor/limit errors,
filtering before limiting, restart persistence, UUID stability after edits and
no missing/duplicate keys. Multi-user tests exercise the new endpoints through
the authenticated backend. Existing adapter tests cover version races, conflicts,
partial saves, reservation races and retry behavior.

Run:

```sh
cargo fmt --all -- --check
cargo clippy -p rusty_mill_board -p rusty_tick --all-targets --all-features -- -D warnings
cargo test -p rusty_mill_board -p rusty_tick
python3 .github/scripts/changes.py check
```

## Production integrity audit and deployment

The read-only storage audit found all parents **#845–851**, all confirmed
children **#959–977** with nonempty prompts, parent links, dependencies and
milestones, and additional C07 children **#978–980**. Those three have parent
**#851**, nonempty prompts and a milestone; dependencies are respectively
`[#850,#849]`, `[#978]`, `[#979]`. This establishes persistence, not whether the
intended C07 workstream is complete. No cards were recreated. Parent cards
#845–847 and #851 have empty prompt fields; this was observed, not repaired.

**Production recovery has not been claimed.** The running binaries remain
unchanged pending independent review, green current-head CI and owner merge.
Agents never queue or merge this PR.

After owner merge, the deployer should:

1. Build both approved binaries with
   `cargo build --release -p rusty_tick -p rusty_mill_board`.
2. Follow the supported stopped-service backup procedure in
   `../../../rusty_tick/deploy/README.md` and `../../deploy/README.md`:
   stop board then tick, copy the **entire** tick data directory and board
   settings into a protected backup, preserve permissions, verify file hashes
   against the stopped originals, and copy the backup off-host. Do not copy
   only `.mmap` files or open a live production data directory with another
   engine process. Verify recovery by opening a separate backup copy on a
   scratch server, checking counts/IDs/relationships; keep the backup intact.
   No such backup or downtime was needed/performed for the read-only diagnosis.
3. Preserve previous binaries, install/restart **tick first**, then install/
   restart board. No environment changes, migration, or schema conversion.
4. Verify the actual MCP connector: `board_whoami`, project-scoped `board_list`,
   `board_milestones`, `board_roadmap`, `card_get`, `card_trace`, `board_events`.
   Use the actual configured lowercase project slug, not the display name.
   Check parents #845–851, children #959–980, full prompt lengths/hashes,
   dependency and milestone IDs against a fresh pre-upgrade baseline. Check
   additional C07 records without creating replacements. Inspect page sizes,
   latency, RSS and service errors while doing so. Health alone is insufficient.
5. Confirm restart persistence and report the C07 intended-vs-persisted scope
   for separate reconciliation. A board restart resets its existing in-memory
   event log; durable move history and stored comments/cards are unaffected.

Rollback: stop board, restore its previous binary, then restore/restart tick's
previous binary if needed, and start board. The additive HTTP change leaves all
persisted formats compatible, so do **not** restore stale data over new writes.
The original board may fail again once the task payload crosses 1 MiB. Retain
backups for disaster recovery rather than treating a data restore as a binary
rollback. Do not reset databases, purge records, or shorten prompts/history.

# Release Notes

The story behind each `rusty_lsp` release — what changed, why it mattered, and
what to watch for when upgrading. For the exhaustive, entry-by-entry technical
log, see [`CHANGELOG.md`](CHANGELOG.md); this page is the narrative version.

---

## Unreleased

Closing LSP 3.17 spec-coverage gaps found by a full audit against the
official specification (method/notification coverage was already 100% —
every remaining gap is at the capability-negotiation/type-modeling layer).
Entries accumulate here until the next version bump.

- Servers can now advertise `workspace.workspaceFolders` support
  (`ServerCapabilities.workspace.workspace_folders`), so clients know
  whether to expect `workspace/didChangeWorkspaceFolders` notifications.
- Eleven more capability-option structs (completion, signature help, code
  action, rename, execute-command, code lens, document link, semantic
  tokens, inlay hint, diagnostic, and workspace-symbol) can now advertise
  work-done-progress support, matching the pattern call/type hierarchy
  already had.
- `ClientCapabilities` gains its first typed sub-tree accessor:
  `.workspace()` parses the client's `workspace` capabilities into a real
  Rust struct instead of dotted-path JSON lookups. The raw JSON escape
  hatch stays exactly as it was — this is purely additive.
- Three more typed accessors: `.window()`, `.general()`, and
  `.notebook_document()`, rounding out every non-`textDocument` capability
  group.
- `.text_document()` starts the `textDocument.*` accessor with its
  core, most-probed groups: sync, completion, hover, signature help,
  the go-to-definition family, references, document highlight, and
  document symbol. The rest of `textDocument.*` follows in later work.
- `.text_document()` grows nine more groups: code action, code lens,
  document link, color provider, the formatting family, rename, folding
  range, selection range, and publish-diagnostics.
- `.text_document()` finishes with its last eight groups (call hierarchy,
  semantic tokens, linked-editing range, moniker, type hierarchy, inline
  value, inlay hint, diagnostic) — every `ClientCapabilities.textDocument.*`
  group the spec defines is now a typed field somewhere on this accessor.
- Dynamic registration (`client/registerCapability`) gets typed builders for
  the methods that previously needed hand-built JSON: `Registration::for_*`
  for semantic tokens, diagnostics, call hierarchy, type hierarchy, and nine
  simpler methods (declaration, type definition, implementation, document
  color, folding range, selection range, moniker, linked editing range,
  inline value) sharing one options shape. The raw-JSON `Registration::new`
  escape hatch still works for anything not covered.
- **Breaking**: the last capability-negotiation gap the audit found —
  16 `ServerCapabilities` fields (`hover_provider`, `definition_provider`,
  `declaration_provider`, `type_definition_provider`,
  `implementation_provider`, `references_provider`,
  `document_highlight_provider`, `document_symbol_provider`,
  `document_formatting_provider`, `document_range_formatting_provider`,
  `folding_range_provider`, `selection_range_provider`, `color_provider`,
  `moniker_provider`, `linked_editing_range_provider`, and
  `inline_value_provider`) were plain `Option<bool>` when the spec allows
  a server to advertise richer per-provider options (mainly
  `workDoneProgress` support, plus a `label` for document symbols). They
  now follow the same `Simple(bool) | Options(...)` pattern the crate
  already used for code actions, rename, workspace symbols, and call/type
  hierarchy. Every new enum implements `From<bool>`, so
  `Some(true)`/`Some(false)` call sites become `Some(true.into())`/
  `Some(false.into())` — a small, mechanical fix. Nothing changes on the
  wire: a `Simple(bool)` value serializes exactly like a bare boolean
  always did.

---

## v0.6.2 — Closing the concurrency gap

*2026-07-17*

A performance-only release. A cross-framework benchmark against `tower-lsp`
and `lsp-server` turned up a hard ceiling on concurrent throughput — flat at
around **6,200 req/s** no matter how many requests a client pipelined at
once. This release finds and removes that ceiling.

**What was wrong.** The server's single writer task flushed every outbound
message on its own: one write, one flush, repeat — even when a dozen
responses from concurrently-finishing handlers were already sitting in the
queue ready to go out together. Throughput was bounded by syscall latency,
not by how fast handlers could actually run.

**What changed.**

- The writer now drains everything already queued into one buffer and issues
  a single write + flush for the whole batch. A lone response still goes out
  immediately — nothing gets held back waiting for company.
- The `in_flight` bookkeeping added in 0.6.1 to prevent a race between a
  handler finishing and its own registration now uses a plain lock held
  across the spawn, instead of a channel round-trip — same guarantee, less
  overhead.

**The result**, same benchmark, same machine:

| Pipelining depth | Before | After | tower-lsp |
|---|---|---|---|
| 8 | 5,744 req/s | 24,437 req/s | 31,125 req/s |
| 32 | 6,105 req/s | 60,137 req/s | 76,375 req/s |
| 128 | 6,182 req/s | **84,195 req/s** | 99,775 req/s |

No public API changed. Upgrade unconditionally.

---

## v0.6.1 — A silent bug, found the hard way

*2026-07-17*

The kind of bug that doesn't announce itself: no panic, no error, no log
line — a request's response just never arrives.

**The setup.** If a handler completes on its very first poll (no genuine
`.await` inside — a cached lookup, a purely computational result), and the
server is under real concurrent load on the multi-threaded runtime, the
handler's task could finish and remove itself from the in-flight table
*before* the spawning thread had finished putting it there in the first
place. Finding nothing to remove, the task concluded a cancellation had
already claimed the request and quietly gave up on sending a response.

**How it was found.** Not from a bug report — from building an honest
cross-framework benchmark. An 8-way concurrent pipelining stress test showed
suspiciously few responses coming back; the race was confirmed independently
outside the crate (a raw Python LSP client, and a standalone Rust driver)
before being traced to this exact line in `spawn_request`.

**The fix.** A structural ordering guarantee — not a statistical
mitigation — so the spawned task cannot touch the in-flight table until the
spawning thread is done registering it.

No public API changes; a pure, unconditional-upgrade bugfix.

---

## v0.6.0 — The mixin sweep, and going wider

*2026-07-17*

Rounds out the last corners of the protocol surface and grows the project's
own tooling.

**Highlights**

- The remaining `workDone`/`partialResult` progress mixins — signature help,
  code lens, document link, document color, color presentation, inlay hint,
  and the full semantic-tokens family — are typed. Every request that the
  spec allows progress reporting on now supports it.
- `Server::with_shutdown_signal(future)`: wind the server down cleanly from
  ctrl-c or a parent-process watchdog, through the normal teardown path
  rather than a bare process kill.
- A `fuzz/` crate (frame parser, message classifier, `Uri`, `apply_edits`)
  running on a weekly schedule, and Criterion benchmarks for the
  position-math hot paths.
- A failed writer (broken pipe, client stopped reading) now tears the
  connection down immediately instead of quietly serving into a void.

---

## v0.5.0 — Progress tokens everywhere, and TCP

*2026-07-17*

**Highlights**

- Client-supplied progress tokens on `hover`, `completion`, `definition`,
  `foldingRange`, and `selectionRange` are no longer silently dropped —
  `Client::begin_progress_for` now works directly from those handlers.
- Typed file-watcher registration: `FileSystemWatcher`, glob patterns
  (plain or relative), and `Registration::for_watched_files`.
- Position-encoding negotiation between client and server is one call.
- `server::serve_tcp` (behind the `tcp` feature): an accept loop serving one
  backend per connection, for editors that speak LSP over a socket instead
  of stdio.
- A release workflow now tags and publishes a GitHub Release automatically
  whenever the crate version changes on `main`.

**Breaking:** `declaration`/`type_definition`/`implementation` take their
own dedicated param structs instead of sharing `TextDocumentPositionParams`.

---

## v0.4.0 — Notifications get their own lane

*2026-07-17*

The headline change: notifications no longer run inline on the message
loop. A slow `didChange` handler used to delay everything behind it —
`$/cancelRequest`, response delivery, the next request in the queue.

**What changed.** Notifications now run in receipt order on a dedicated
serialized worker. Document state stays consistent (a `didChange` is
guaranteed applied before a later request observes the buffer — enforced
via a completion watermark) but the message loop itself is never blocked
by one. A panicking notification handler is now caught and logged instead
of taking the server down with it.

Also added: `LocationLink` support for the goto family, the full
`TextDocumentSyncOptions` capability shape, `SemanticTokensBuilder` for the
spec's relative token encoding, and ordering/range helpers on `Position`
and `Range`.

**Breaking:** notification concurrency model changed as above;
`ServerCapabilities::text_document_sync` is now `Option<TextDocumentSyncCapability>`.

---

## v0.3.0 — Completion and diagnostics grow up

*2026-07-17*

**Highlights**

- `CompletionItem` fully modelled: label details, tags, snippets, text
  edits (including insert/replace), additional edits, resolve data, and
  LSP 3.17 `itemDefaults`.
- `Diagnostic` gains its rich fields — `tags`, `relatedInformation`,
  `codeDescription`, `data` — completing the diagnostic → quick-fix round
  trip.
- Typed dynamic registration and typed `WorkspaceEdit::document_changes`
  (versioned edits, file creates/renames/deletes, change annotations).
- `text::apply_edits`: atomic, overlap-checked batch edit application.

**Breaking:** `exit` without a prior `shutdown` now makes `serve` return an
error (so a `Result`-returning `main` exits with code 1, per spec); several
public structs gained fields, so literals need `..Default::default()`.

---

## v0.2.0 — Cancellation, concurrency limits, and a real `Uri`

*2026-07-17*

**Highlights**

- Cooperative cancellation: `rusty_lsp::cancel::current()` exposes a token
  tripped by `$/cancelRequest` before the task abort even lands — for work
  an abort can't reach, like a `spawn_blocking` computation.
- `Server::with_max_concurrent_requests` and `Server::with_outbound_queue_limit`
  put real bounds on a misbehaving or overwhelmed client.
- `testing::TestClient`, the in-memory test harness, ships for the first
  time.
- Handler panics are now caught inside the task, instead of racing a
  separate watcher.

**Breaking:** `Uri` becomes a normalizing newtype instead of a bare
`String` alias — differently-spelled equivalents now hash and compare
equal.

---

## v0.1.1 — Baseline

*The starting point.* JSON-RPC framing and dispatch, the `LanguageServer`
trait with typed handlers for the core protocol, lifecycle enforcement,
abort-based cancellation, the `Client` handle, an optional `Documents`
store, and UTF-8/16/32 position conversion utilities.

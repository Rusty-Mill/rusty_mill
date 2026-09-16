# Changelog

All notable changes to `rusty_lsp` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) (0.x: minor bumps may break).
See [`RELEASE_NOTES.md`](RELEASE_NOTES.md) for the narrative version of
each release.

## [Unreleased]

### Added

- `WorkspaceServerCapabilities::workspace_folders`
  (`WorkspaceFoldersServerCapabilities { supported, change_notifications }`):
  advertise multi-root workspace support and interest in
  `workspace/didChangeWorkspaceFolders` notifications, matching the LSP 3.17
  `ServerCapabilities.workspace.workspaceFolders` shape. Previously
  `WorkspaceServerCapabilities` only modeled `file_operations`.
- `work_done_progress: Option<bool>` on `CompletionOptions`,
  `SignatureHelpOptions`, `CodeActionOptions`, `RenameOptions`,
  `ExecuteCommandOptions`, `CodeLensOptions`, `DocumentLinkOptions`,
  `SemanticTokensOptions`, `InlayHintOptions`, `DiagnosticOptions`, and
  `WorkspaceSymbolOptions` — the same `WorkDoneProgressOptions` mixin
  `CallHierarchyOptions`/`TypeHierarchyOptions` already had, now applied
  uniformly across every capability option struct the spec extends it to.
- `ClientCapabilities::workspace()`: a typed accessor parsing the
  `workspace` sub-tree of the client's capabilities into
  `WorkspaceClientCapabilities` (new module `lsp::client_capabilities`),
  covering `applyEdit`, `workspaceEdit` (including resource-operation and
  failure-handling support, change-annotation grouping), dynamic
  registration for `didChangeConfiguration`/`didChangeWatchedFiles`
  (including relative-pattern support), `workspace/symbol` capabilities
  (symbol-kind/tag/resolve support), `workspaceFolders`/`configuration`,
  the `semanticTokens`/`codeLens`/`inlineValue`/`inlayHint`/`diagnostics`
  refresh-support flags, and file-operation capabilities. `raw` and the
  existing dotted-path `get`/`supports` helpers are unchanged — this is a
  purely additive, typed convenience layer on top of the same data.
- `ClientCapabilities::window()`, `::general()`, and `::notebook_document()`:
  the same typed-accessor treatment for `window` (`workDoneProgress`,
  `showMessageRequest`/`showDocument` capabilities), `general`
  (`staleRequestSupport`, the client's regex engine/Markdown parser,
  `positionEncodings`), and `notebookDocument.synchronization`.
- `enums::InsertTextMode` (LSP 3.16): `AsIs`/`AdjustIndentation`, needed to
  model completion's insert-text-mode capability faithfully.
- `ClientCapabilities::text_document()`: a typed accessor for the core
  `textDocument.*` capability groups — `synchronization`, `completion`
  (including per-item snippet/commit-character/tag/resolve/insert-mode
  support and `completionList.itemDefaults`), `hover`, `signatureHelp`,
  the `declaration`/`definition`/`typeDefinition`/`implementation` "go to"
  family, `references`, `documentHighlight`, and `documentSymbol`. The
  remaining `textDocument.*` groups (code action, code lens, semantic
  tokens, etc.) are tracked as follow-up work; `raw`/`get`/`supports` are
  unchanged.
- `enums::PrepareSupportDefaultBehavior` (LSP 3.16): `Identifier`, needed to
  model `textDocument.rename`'s `prepareSupportDefaultBehavior` capability.
- `TextDocumentClientCapabilities` grows nine more groups: `codeAction`
  (literal-kind/preferred/disabled/data/resolve support, change-annotation
  honoring), `codeLens`, `documentLink` (including tooltip support),
  `colorProvider`, `formatting`/`rangeFormatting`/`onTypeFormatting`,
  `rename` (prepare support and its default-behavior variants),
  `foldingRange` (range limit, line-folding-only, kind/collapsed-text
  support), `selectionRange`, and `publishDiagnostics` (related
  information, tag/version/code-description/data support). First of two
  follow-ups extending `ClientCapabilities::text_document()` beyond its
  core set; the remaining groups (call hierarchy, semantic tokens,
  linked-editing range, moniker, type hierarchy, inline value, inlay
  hint, diagnostic) follow separately.
- `TextDocumentClientCapabilities` grows its remaining eight groups:
  `callHierarchy`, `semanticTokens` (full `requests.range`/`requests.full`
  union shapes, token types/modifiers/formats, overlapping/multiline token
  support, `serverCancelSupport`, `augmentsSyntaxTokens`),
  `linkedEditingRange`, `moniker`, `typeHierarchy`, `inlineValue`,
  `inlayHint` (resolve support), and `diagnostic` (related-document
  support) — this completes `ClientCapabilities.textDocument.*` coverage.
  Also adds `client_capabilities::TokenFormat`/`token_format` (an open
  string enum, matching the existing `CodeActionKind`/`FoldingRangeKind`
  pattern) and `EmptyCapability` (the spec's `{}` object form where a
  capability is `boolean | {}`).
- Typed `*RegistrationOptions` and `Registration::for_*` builders for the
  remaining dynamically-registerable methods: a shared
  `SimpleRegistrationOptions` (plus spec-named type aliases
  `DeclarationRegistrationOptions`, `TypeDefinitionRegistrationOptions`,
  `ImplementationRegistrationOptions`, `DocumentColorRegistrationOptions`,
  `FoldingRangeRegistrationOptions`, `SelectionRangeRegistrationOptions`,
  `MonikerRegistrationOptions`, `LinkedEditingRangeRegistrationOptions`,
  `InlineValueRegistrationOptions`) for the nine methods whose only
  registration option is `workDoneProgress` support, built via
  `Registration::for_documents_with_options`; and dedicated
  `SemanticTokensRegistrationOptions`/`DiagnosticRegistrationOptions`/
  `CallHierarchyRegistrationOptions`/`TypeHierarchyRegistrationOptions`
  with matching `Registration::for_semantic_tokens`/`for_diagnostic`/
  `for_call_hierarchy`/`for_type_hierarchy` builders. `Registration::new`
  (the raw-JSON escape hatch) is unchanged and still works for anything
  not covered here.
- `WorkDoneProgressOptions` (in `progress`): the `{ workDoneProgress?:
  boolean }` mixin the spec applies to many `ServerCapabilities` provider
  option shapes, shared by the 15 provider-capability enums below rather
  than duplicated per method.
- `DocumentSymbolOptions` (in `symbols`): `work_done_progress` plus
  `label` (LSP 3.16), the options shape for `documentSymbolProvider`.

### Changed

- **Breaking**: 16 `ServerCapabilities` fields that were `Option<bool>`
  are now `Option<XProviderCapability>`, an untagged `Simple(bool) |
  Options(...)` enum — matching the spec's `boolean | XOptions` shape and
  the pattern already used by `code_action_provider`/`rename_provider`/
  `workspace_symbol_provider`/`call_hierarchy_provider`/
  `type_hierarchy_provider`. Affected: `hover_provider`
  (`HoverProviderCapability`), `definition_provider`
  (`DefinitionProviderCapability`), `declaration_provider`
  (`DeclarationProviderCapability`), `type_definition_provider`
  (`TypeDefinitionProviderCapability`), `implementation_provider`
  (`ImplementationProviderCapability`), `references_provider`
  (`ReferencesProviderCapability`), `document_highlight_provider`
  (`DocumentHighlightProviderCapability`), `document_symbol_provider`
  (`DocumentSymbolProviderCapability`, the one with an extra `label`
  field), `document_formatting_provider`
  (`DocumentFormattingProviderCapability`),
  `document_range_formatting_provider`
  (`DocumentRangeFormattingProviderCapability`), `folding_range_provider`
  (`FoldingRangeProviderCapability`), `selection_range_provider`
  (`SelectionRangeProviderCapability`), `color_provider`
  (`ColorProviderCapability`), `moniker_provider`
  (`MonikerProviderCapability`), `linked_editing_range_provider`
  (`LinkedEditingRangeProviderCapability`), and `inline_value_provider`
  (`InlineValueProviderCapability`). `inline_completion_provider` is
  **not** included — it tracks LSP 3.18, which is still proposed.
  Migration: `Some(true)`/`Some(false)` literals become
  `Some(true.into())`/`Some(false.into())` (every new enum implements
  `From<bool>`), or match on `Simple`/`Options` directly. The wire format
  is unchanged: a `Simple(bool)` value serializes identically to a bare
  `true`/`false`, so this is a Rust-API-only break, not a protocol one.

## [0.6.2] — 2026-07-17

### Fixed

- **Performance**: concurrent request throughput plateaued far below what
  the runtime could sustain (~6.2K req/s regardless of pipelining depth,
  versus tower-lsp's 32.7K–121.5K req/s at the same depths in a head-to-head
  benchmark). Root cause: the single writer task wrote and `flush()`ed
  every outbound message individually — one syscall pair per message, fully
  serialized — so a burst of responses from many concurrently-finishing
  handlers still drained the outbound queue one flush at a time. Fixed by
  draining every message already queued (via `try_recv`, which never waits)
  into one buffer before issuing a single `write_all` + `flush` for the
  whole batch; a lone message still gets a `write_all` + `flush` immediately
  with no added latency. Depth-128 pipelined throughput went from ~6.2K to
  ~84K req/s in the same benchmark, closing nearly the entire gap to
  tower-lsp. No public API changes.
- Replaced the `oneshot::channel` "start gate" `spawn_request` used (added
  in 0.6.1 to close the silent-response-drop race) with a cheaper
  equivalent: the `in_flight` lock is now held across the duplicate-id
  check, the `tokio::spawn` call, and the subsequent insert, as one
  critical section. If the spawned task's own thread reaches its
  `in_flight` removal before the spawning thread's insert, it now simply
  blocks briefly on the same `std::sync::Mutex` instead of waiting on a
  channel wakeup — same hard ordering guarantee, without a mandatory extra
  scheduler round-trip on every request. (This alone had negligible
  standalone effect on the plateau above; the writer batching fix was the
  actual fix. Both are shipped together since they touch the same code
  path and were investigated in the same pass.)

## [0.6.1] — 2026-07-17

### Fixed

- **Critical**: a request whose handler has no genuine `.await` inside it
  (returns on its very first poll — the common case for a trivial,
  cached, or purely computational result) could have its response
  **silently dropped** under real concurrent load. `spawn_request` called
  `tokio::spawn` before registering the task in `in_flight`; on the
  multi-threaded runtime, the spawned task could run to completion **on
  another worker thread** and reach its own removal-from-`in_flight`
  before the spawning thread finished inserting that same entry. Finding
  nothing to remove, it concluded (incorrectly) that a `$/cancelRequest`
  had already claimed the request, and skipped sending a response —
  with no error, no panic, no log: the client's request simply never
  answers. Found via cross-framework benchmarking (a 8-way concurrent
  pipelining stress test); reproduced independently outside the crate
  (a raw Python LSP client, and a separate Rust driver) before being
  traced to this exact race. Fixed with a start-gate (`oneshot::channel`)
  that structurally prevents the spawned task from touching `in_flight`
  until the spawning thread has finished inserting its entry — not a
  statistical mitigation, a hard ordering guarantee. No public API
  changes; pure bugfix, safe to upgrade from 0.6.0 unconditionally.

## [0.6.0] — 2026-07-17

### Added

- The remaining `workDone`/`partialResult` mixins: `SignatureHelpParams`,
  `CodeLensParams`, `DocumentLinkParams`, `DocumentColorParams`,
  `ColorPresentationParams`, `InlayHintParams`, and the
  `SemanticTokens*Params` family — the mixin sweep is complete.
- `DocumentSymbol::tags` and `SymbolInformation::tags` (the modern
  deprecation form).
- `Server::with_shutdown_signal(future)`: external termination (ctrl-c,
  parent-process watchdogs) through the normal teardown path.
- `TestClient::spawn_configured(configure, build)`: exercise `Server`
  builder options from the exported harness.
- `Client::log`/`log_debug`/`log_info`/`log_warning`/`log_error`
  shortcuts.
- `Uri::parent()` and `Uri::join(segment)` path helpers
  (percent-encoding-aware).
- A `fuzz/` crate with cargo-fuzz targets (frame parser, message
  classifier, `Uri`, `apply_edits`) and a weekly fuzz workflow.
- Criterion benchmarks (`cargo bench`): `LineIndex` vs the free
  conversion functions, batch edit application, framing round trips.
- Feature-gated APIs are labeled on docs.rs via `doc_cfg`.

### Fixed

- A failed writer (client stopped reading; broken pipe) now tears the
  connection down promptly instead of serving into a void until reader
  EOF; the writer's io error is what `serve` returns.

## [0.5.0] — 2026-07-17

### Added

- Dedicated `DeclarationParams`/`TypeDefinitionParams`/`ImplementationParams`,
  and the spec's `workDone`/`partialResult` mixins on `HoverParams`,
  `CompletionParams`, `DefinitionParams`, `FoldingRangeParams`, and
  `SelectionRangeParams` — client progress tokens are no longer silently
  dropped, and `Client::begin_progress_for` works from those handlers.
- `CompletionOptions::all_commit_characters` and
  `completion_item.labelDetailsSupport` (via
  `CompletionOptionsCompletionItem`).
- Typed file-watcher registration: `FileSystemWatcher`, `GlobPattern`
  (plain or `RelativePattern`), `watch_kind` flags,
  `DidChangeWatchedFilesRegistrationOptions`, and
  `Registration::for_watched_files`.
- Position-encoding negotiation:
  `ClientCapabilities::position_encodings()` and
  `negotiate_position_encoding(preference)`.
- `server::serve_tcp(listener, factory)` (behind the `tcp` feature): a
  multi-connection accept loop, one backend per connection.
- `Server::with_teardown_grace(Duration)` (default 2s): bounds how long
  teardown waits for still-queued notification handlers on abrupt endings.
- A release workflow that tags `vX.Y.Z` and publishes a GitHub Release
  (with the matching changelog section) whenever the crate version changes
  on `main`; a pinned-MSRV (1.85) CI job; `docs.rs` builds with all
  features.
- Deterministic property tests: `LineIndex` vs the free conversion
  functions, `apply_edits` vs a naive reference, and transport round trips
  at random payload sizes.

### Fixed

- `Documents` lookups by raw string now fall back to the normalized URI
  spelling, so `get("FILE:///a")` finds a document stored under
  `file:///a`.
- `LocationLink` is now re-exported from `rusty_lsp::lsp` (it was only
  reachable via `lsp::features` in 0.4.0).
- Teardown no longer waits unboundedly for a slow queued notification
  handler after `exit`/EOF.

### Changed

- **Breaking**: `declaration`/`type_definition`/`implementation` take
  their dedicated param structs instead of `TextDocumentPositionParams`;
  fields were added to several param structs (use `..Default::default()`
  in literals).

## [0.4.0] — 2026-07-17

### Added

- `textDocument/documentHighlight` gains `LocationLink` company: the goto
  family (`definition`/`declaration`/`typeDefinition`/`implementation`) can
  now return `GotoDefinitionResponse::Links(Vec<LocationLink>)` for clients
  advertising `linkSupport`.
- `TextDocumentSyncOptions`/`SaveOptions`: the full form of the
  `textDocumentSync` capability (open/close, will-save, save-with-text).
- `SemanticTokensBuilder`: builds the spec's relative-encoded token array
  from absolute positions, with legend name resolution, document-order
  sorting, and multi-line range splitting (`push_range`).
- `Documents::uris()`, `len()`, `is_empty()`, and `for_each()` for
  enumerating the open set (e.g. for `workspace/diagnostic`).
- `Client::begin_progress_for(&WorkDoneProgressParams, …)`: progress on the
  client-supplied `workDoneToken`, no `create` round trip.
- `Client::config_section::<T>(section, scope)`: one-section, typed
  `workspace/configuration`.
- `Position` implements `PartialOrd`/`Ord` (line, then character);
  `Range::contains`, `Range::overlaps`, `Range::intersection` (half-open
  semantics).
- `TestClient::open`/`change_full`/`close` document-lifecycle helpers.
- Direct unit tests for the transport framing layer.
- The example server implements `documentHighlight` via
  `Documents::offset_at`.

### Changed

- **Breaking**: notifications now run on a dedicated serialized worker
  instead of inline on the message loop. Receipt order is preserved, and
  requests still observe every notification received before them (they
  synchronize on a completion watermark) — but `$/cancelRequest` handling
  and response delivery are no longer delayed by slow notification
  handlers. A panicking notification handler no longer tears down the
  server; it is caught and logged.
- **Breaking**: `ServerCapabilities::text_document_sync` is now
  `Option<TextDocumentSyncCapability>` (`Kind | Options`); a bare
  `TextDocumentSyncKind` converts with `.into()`.
- `shutdown` waits for already-received notifications to land before the
  backend's `shutdown` handler runs.

## [0.3.0] — 2026-07-17

### Added

- Typed `textDocument/documentHighlight` (method, types, capability).
- Fully modelled `CompletionItem` (label details, tags, snippets,
  `textEdit`/`InsertReplaceEdit`, `additionalTextEdits`, `command`, resolve
  `data`) and `CompletionList::item_defaults` (LSP 3.17).
- `Diagnostic` rich fields: `tags`, `relatedInformation`,
  `codeDescription`, `data`, with builder helpers.
- Typed dynamic registration: `DocumentFilter`/`DocumentSelector`/
  `TextDocumentRegistrationOptions`, `Registration::for_documents`.
- Typed `WorkspaceEdit::document_changes`: `TextDocumentEdit`,
  `CreateFile`/`RenameFile`/`DeleteFile`, `changeAnnotations`.
- `Error::content_modified()`, `Error::server_cancelled()`,
  `Error::request_failed()`.
- `Documents::offset_at`/`position_at`/`with_index` over a per-document
  cached `LineIndex`.
- `text::apply_edits`/`apply_edits_with`: atomic, overlap-checked batch
  edit application.
- `TestClient` receive timeouts (default 10s, `with_timeout`).
- CI runs `--all-features` and builds docs with `-D warnings`;
  `rust-version = "1.85"` and `readme` declared in the manifest.

### Changed

- **Breaking**: `CompletionItem::documentation` is now
  `Option<Documentation>` (string or markup).
- **Breaking**: `exit` without a prior `shutdown` makes `Server::serve`
  return an error, so a `Result` main exits with code 1 per the spec.
- **Breaking**: fields were added to public structs (`CompletionItem`,
  `Diagnostic`, `CompletionList`, `WorkspaceEdit`); struct literals must
  use `..Default::default()` or the builders.

## [0.2.0] — 2026-07-17

### Added

- Typed methods for `textDocument/moniker`, `linkedEditingRange`,
  `inlineValue`, and `inlineCompletion` (3.18 proposed), with matching
  capabilities.
- `workspace/symbol` 3.17 form (`WorkspaceSymbol`, URI-only locations) and
  `workspaceSymbol/resolve`.
- `Client` helpers: `workspace_folders()`, `telemetry_event()`,
  `refresh_folding_ranges()`, `refresh_inline_values()`,
  `send_request_with_timeout`, `begin_progress` (RAII `ProgressGuard`).
- Cooperative cancellation: `rusty_lsp::cancel::current()` exposes a
  `CancelToken` tripped by `$/cancelRequest` before the task abort.
- `Server::with_max_concurrent_requests` and
  `Server::with_outbound_queue_limit`.
- `Documents::with_encoding`, `Documents::with`, stale-version guarding;
  `text::LineIndex`.
- `testing::TestClient` in-memory harness; `tcp` and `tracing` cargo
  features; `InitializeParams::workspace_roots()`.

### Changed

- **Breaking**: `Uri` is a normalizing newtype (was a `String` alias).
- **Breaking**: `symbol()` returns `Option<WorkspaceSymbolResponse>`;
  `workspace_symbol_provider` accepts `bool | WorkspaceSymbolOptions`.
- Handler panics are caught in-task; panic responses no longer race a
  watcher task.

## [0.1.1] — baseline

Initial framework: JSON-RPC framing and dispatch, the `LanguageServer`
trait with typed handlers for the core protocol, lifecycle enforcement,
abort-based request cancellation, `Client` handle, optional `Documents`
store, and UTF-8/16/32 position conversion utilities.

# rusty_lsp

A small, reusable [Language Server Protocol][lsp] framework for async Rust.

`rusty_lsp` owns the protocol plumbing — JSON-RPC framing, message dispatch, the
`initialize`/`shutdown` lifecycle, and request cancellation — and hands you a
single trait, [`LanguageServer`](src/service.rs), to implement your language's
behaviour.

It is **not** a server for any particular language; it is the reusable engine you
build one on top of.

- **vs. hand-rolled JSON-RPC** — you get typed handlers and lifecycle correctness
  for free.
- **vs. a `tower`-based stack** — the dependency footprint is just `tokio`,
  `serde`, and `serde_json`. No `async-trait`, no `tower`.

## Design at a glance

| Concern | How `rusty_lsp` handles it |
|---|---|
| **Extension point** | One trait, `LanguageServer`. Every method except `initialize` has a default, so a minimal server is just `initialize` returning its capabilities. |
| **Async without `async-trait`** | Trait methods are declared `-> impl Future + Send` (RPITIT). You still write the bodies as ordinary `async fn`. The `+ Send` bound lets request handlers be spawned on a multi-threaded runtime with no boxing layer. |
| **Concurrency** | Notifications run in receipt order on a dedicated serialized worker (so document state stays consistent — a `didChange` is applied before a later request observes the buffer, enforced via a completion watermark) without ever blocking the message loop: `$/cancelRequest` and response delivery stay responsive even mid-`didChange`. Requests are spawned tasks; `Server::with_max_concurrent_requests` optionally caps how many handler bodies run at once, and `Server::with_outbound_queue_limit` bounds the output queue against a client that stops reading. |
| **Cancellation** | `$/cancelRequest` aborts the in-flight handler and replies with `RequestCancelled` (`-32800`). The bookkeeping guarantees each request is answered **exactly once**, even when a handler completes at the same instant a cancel arrives. Handlers additionally see a cooperative [`CancelToken`](src/cancel.rs) (via `rusty_lsp::cancel::current()`) that reaches work an abort cannot: `spawn_blocking` computations, helper tasks, CPU-bound stretches. |
| **Lifecycle** | External termination is one builder call — `Server::with_shutdown_signal(future)` winds down cleanly on ctrl-c or a parent-process watchdog. `initialize` is enforced first; requests before it get `ServerNotInitialized` (`-32002`); a second `initialize` is rejected; work after `shutdown` is refused; `exit` (or EOF at a frame boundary) stops the loop cleanly — and `exit` *without* a prior `shutdown` makes `serve` return an error, so a `fn main() -> Result<()>` exits with code 1 exactly as the spec requires. |
| **Extensibility** | Unmodelled methods reach `handle_request` / `handle_notification`, and `ServerCapabilities::extra` lets you advertise any capability the framework does not type. |

## Architecture

| Module | Responsibility |
|---|---|
| [`transport`](src/transport.rs) | `Content-Length` framing over any async byte stream |
| [`jsonrpc`](src/jsonrpc.rs) | JSON-RPC 2.0 request/response/notification model |
| [`lsp`](src/lsp/mod.rs) | Typed LSP protocol data structures |
| [`text`](src/text.rs) | UTF-16 ↔ byte position conversions for buffer indexing |
| [`service`](src/service.rs) | The `LanguageServer` trait you implement |
| [`client`](src/client.rs) | The `Client` handle for server → client messages |
| [`server`](src/server.rs) | The `Server` runtime: dispatch, lifecycle, cancellation |
| [`documents`](src/documents.rs) | Optional managed store of open document text |

## Adding the dependency

`rusty_lsp` distributes via pinned git tags, not crates.io. Depend on a
tagged release:

```toml
[dependencies]
rusty_lsp = { git = "https://github.com/baileyrd/rusty_lsp", tag = "v0.6.2" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

(A path dependency works too during local development: `rusty_lsp = { path = "../rusty_lsp" }`.)

Requires a Rust toolchain supporting **edition 2024** (Rust 1.85+).

## Quick start

A minimal server that advertises hover support:

```rust
use rusty_lsp::error::Result;
use rusty_lsp::lsp::{
    InitializeParams, InitializeResult, ServerCapabilities, ServerInfo, TextDocumentSyncKind,
};
use rusty_lsp::{Client, LanguageServer, Server};

struct Backend {
    client: Client,
}

impl LanguageServer for Backend {
    // Declared `-> impl Future + Send` on the trait; written as plain `async fn`.
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncKind::Full.into()),
                hover_provider: Some(true.into()),
                ..Default::default()
            },
            server_info: Some(ServerInfo { name: "demo".into(), version: None }),
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // `serve` hands you a `Client`; store it so handlers can talk back
    // (diagnostics, log/show-message, server → client requests).
    Server::stdio().serve(|client| Backend { client }).await
}
```

### Talking back to the editor

The `Client` is your outbound channel. It is cheap to clone and safe to share
across tasks:

```rust,ignore
// Notifications (fire-and-forget):
client.publish_diagnostics(uri, diagnostics, Some(version))?;
client.log_info("indexing complete")?;  // also log/log_debug/log_warning/log_error

// A server → client request, awaiting the reply:
let config: MyConfig = client
    .send_request("workspace/configuration", params)
    .await?;

// Typed helpers for the common server -> client requests:
client.show_message_request(MessageType::Info, "Retry?", actions).await?;
client.show_document(ShowDocumentParams { uri, ..Default::default() }).await?;
client.register_capability(vec![Registration::new("1", "textDocument/formatting", None)]).await?;
client.unregister_capability(vec![Unregistration { id: "1".into(), method: "textDocument/formatting".into() }]).await?;
client.workspace_folders().await?;      // workspace/workspaceFolders
client.telemetry_event(payload)?;       // telemetry/event

// One-section, typed configuration:
let settings: MySettings = client.config_section("myServer", None).await?;

// Progress on the token the client attached to a request (no create round trip):
let progress = client.begin_progress_for(&params.work_done, begin)?;

// Don't let a wedged editor hang a handler forever:
let cfg: Vec<Value> = client
    .send_request_with_timeout("workspace/configuration", params, Duration::from_secs(5))
    .await?;

// Leak-proof work-done progress (`end` is sent even on early return / panic):
let progress = client.begin_progress("indexing", WorkDoneProgressBegin {
    title: "Indexing".into(), ..Default::default()
})?;
progress.report(WorkDoneProgressReport { percentage: Some(50), ..Default::default() })?;
progress.finish(WorkDoneProgressEnd { message: Some("done".into()) })?;
```

### Cancellation that reaches everything

`$/cancelRequest` aborts the handler task automatically. For work an abort
cannot reach — `spawn_blocking`, helper tasks, long CPU-bound loops — every
request handler also runs inside a cooperative token scope:

```rust,ignore
async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
    let token = rusty_lsp::cancel::current().unwrap_or_default();
    let hits = tokio::task::spawn_blocking(move || {
        big_index.scan(|item| !token.is_cancelled() /* keep going? */)
    }).await?;
    Ok(Some(hits))
}
```

### Managing document text

[`Documents`](src/documents.rs) is an optional, concurrency-safe store of
open document text that applies `didOpen`/`didChange`/`didClose` for you,
including incremental (range) edits, so a backend doesn't have to hand-roll
a `HashMap<Uri, String>`:

```rust,ignore
struct Backend {
    client: Client,
    documents: Documents,
}

async fn did_open(&self, params: DidOpenTextDocumentParams) {
    self.documents.did_open(&params).await;
}

async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri = &params.text_document_position.text_document.uri;
    let Some(doc) = self.documents.get(uri).await else { return Ok(None) };
    // ... inspect doc.text ...
}
```

`Documents` is position-encoding-aware (`Documents::with_encoding` for
servers that negotiated UTF-8/UTF-32), guards against replayed stale
`didChange` versions, and offers a borrow-based accessor for hot paths:

```rust,ignore
// No full-text clone per request:
let hover = self.documents.with(&uri, |doc| hover_at(&doc.text, position)).await;
```

It's entirely optional — wire up the matching `LanguageServer` methods only
if you want it; the framework doesn't require or assume it exists.

For semantic tokens, `SemanticTokensBuilder` computes the spec's
relative encoding (`deltaLine`/`deltaStart`) from absolute positions —
legend names resolved, tokens sorted into document order, multi-line
ranges split per line — so servers never hand-roll the most error-prone
encoding in LSP.

`Documents` also exposes encoding-aware position math directly —
`offset_at`/`position_at`/`with_index` resolve through a per-document cached
[`text::LineIndex`](src/text.rs) (invalidated on every edit), so the common
"where is this cursor in the buffer?" lookup is one call and `O(log n)` —
and the open set is enumerable (`uris`, `len`, `for_each`) for
workspace-wide operations.

URIs are a lightweight [`Uri`](src/lsp/base.rs) newtype that normalizes on
construction (scheme case, percent-encoding hex), so differently-spelled
equivalents hash and compare equal, with `Uri::from_file_path` /
`Uri::to_file_path` for filesystem conversion. For position math at scale,
[`text::LineIndex`](src/text.rs) turns per-conversion `O(document)` scans
into a binary search over precomputed line starts, and
[`text::apply_edits`](src/text.rs) applies a batch of `TextEdit`s atomically
(any order, overlap-checked) — handy for asserting formatting/rename results
in tests.

Dynamic registration is fully typed — document selectors
(`Registration::for_documents`) and file watchers
(`Registration::for_watched_files` with `FileSystemWatcher`/`watch_kind`)
— and position-encoding negotiation is one call:
`capabilities.negotiate_position_encoding(&[PositionEncodingKind::Utf8])`,
pairing with `Documents::with_encoding`.

Diagnostics carry the full 3.16 surface (`tags`, `relatedInformation`,
`codeDescription`, `data` for the diagnostic→quick-fix round trip), and
`WorkspaceEdit` models `documentChanges` — versioned edits, file
creates/renames/deletes, and change annotations — as real types. Dynamic
registrations get typed `DocumentSelector`/`DocumentFilter` options via
`Registration::for_documents`.

A backend can also inspect what the client declared support for via
`ClientCapabilities::get`/`supports`, which walk the raw capabilities object
by dotted path (e.g. `capabilities.supports("workspace.applyEdit")`) without
requiring every capability leaf to have its own typed field.

### Handling methods the framework doesn't model

Core navigation and editing requests (`hover`, `completion` + resolve
(with fully modelled items: snippets, text edits, label details, tags,
resolve `data`, 3.17 `itemDefaults`),
`definition`/`declaration`/`typeDefinition`/`implementation` (returning
plain locations or `LocationLink`s for `linkSupport` clients), `references`,
`documentHighlight`, `documentSymbol`, `workspace/symbol`, `signatureHelp`,
`codeAction` + resolve, `rename` + `prepareRename`, `workspace/executeCommand`), editor-UX requests
(`formatting`/`rangeFormatting`/`onTypeFormatting`, `foldingRange`,
`selectionRange`, `codeLens` + resolve, `documentLink` + resolve,
`documentColor`/`colorPresentation`, `semanticTokens` full/delta/range,
`inlayHint` + resolve), diagnostics (push via `publishDiagnostics`, and the
pull model via `textDocument/diagnostic`/`workspace/diagnostic`), file
lifecycle (`willSave`/`willSaveWaitUntil`, `will`/`didCreateFiles`,
`will`/`didRenameFiles`, `will`/`didDeleteFiles`), and notifications
(`didOpen`/`didChange`/`didClose`/`didSave`,
`workspace/didChangeConfiguration`/`didChangeWatchedFiles`/`didChangeWorkspaceFolders`,
`notebookDocument/didOpen`/`didChange`/`didSave`/`didClose`),
call hierarchy (`prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`),
type hierarchy (`prepareTypeHierarchy`, `supertypes`, `subtypes`),
`workspace/symbol` + `workspaceSymbol/resolve` (both the flat and 3.17
lazily-resolved forms), `textDocument/moniker`, linked editing ranges,
inline values (`textDocument/inlineValue`), inline completions
(`textDocument/inlineCompletion`, 3.18 proposed), and
`$/setTrace` (paired with [`Client::log_trace`](src/client.rs) for
`$/logTrace`) have typed trait methods. `references`, `workspace/symbol`,
`documentSymbol`, `formatting`, and `codeAction` also accept the spec's
`workDoneToken` / `partialResultToken` progress mixins, streamable via
[`Client::send_progress`](src/client.rs). For anything else — `textDocument/documentHighlight`,
proposed 3.18 methods, and so on — override the escape hatches and advertise
the capability through `ServerCapabilities::extra`:

```rust,ignore
async fn handle_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
    match method {
        "textDocument/documentHighlight" => { /* deserialize params, return a JSON result */ }
        other => Err(Error::method_not_found(other.to_owned())),
    }
}
```

## Cargo features

| Feature | Adds |
|---|---|
| `tcp` | `Server::from_tcp` (single connection) and `server::serve_tcp` (accept loop, one backend per connection) |
| `tracing` | Wire-level `tracing` instrumentation of the message loop |

## Example server

[`examples/text_server.rs`](examples/text_server.rs) is a complete, runnable
backend for plain-text documents. It tracks open buffers with
[`Documents`](src/documents.rs) (including incremental edits) and provides:

- **diagnostics** — flags `TODO` / `FIXME` / `XXX` markers, republished on each edit;
- **hover** — the word under the cursor and how often it occurs;
- **completion** — the distinct words already present in the buffer.

It dogfoods the [`text`](src/text.rs) module to convert the protocol's UTF-16
columns to byte offsets, so multi-byte characters on a line do not skew ranges.

Run it as an editor would (it speaks LSP over stdio):

```sh
cargo run --example text_server
```

## Testing

The crate ships a first-class test harness, [`testing::TestClient`](src/testing.rs),
that plays the editor's role over in-memory pipes, so backend tests exercise
the full stack (framing, dispatch, lifecycle, cancellation) with typed
requests:

```rust,ignore
let mut client = TestClient::spawn(|client| Backend { client });
client.initialize(InitializeParams::default()).await?;
let hover: Option<Hover> = client.request("textDocument/hover", params).await?;
client.shutdown_and_exit().await?;
```

Every receive has a default 10s timeout (configurable via `with_timeout`),
so a dropped message fails the test with a descriptive error — including
what was buffered while scanning — instead of hanging the suite.

For the crate's own suite:

```sh
cargo test --all-features
cargo clippy --all-targets --all-features
cargo fmt --check
```

The suite covers JSON-RPC wire shapes, integer/string enum encoding, UTF-16
position math (including multi-byte characters), and end-to-end server behaviour
over in-memory pipes: capability negotiation, lifecycle rejection, diagnostics
publishing, hover/completion dispatch, invalid-params and method-not-found
errors, request cancellation, and clean shutdown/exit/EOF teardown.

## Release notes

See [`RELEASE_NOTES.md`](RELEASE_NOTES.md) for the story behind each
release, or [`CHANGELOG.md`](CHANGELOG.md) for the entry-by-entry technical
log.

## License

Dual-licensed under `MIT OR Apache-2.0` (see the `license` field in
[`Cargo.toml`](Cargo.toml)), at your option.

[lsp]: https://microsoft.github.io/language-server-protocol/

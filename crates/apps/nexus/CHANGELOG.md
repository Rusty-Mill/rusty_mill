# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions
follow the workspace version in `Cargo.toml`. Started 2026-06-10 (V18,
`docs/0.1.2/audits/repo-review-2026-06-10.md`) — history before that date
lives in the git log and in `docs/0.1.2/audits/`.

## [Unreleased]

### Added
- **`nexus-storage`: `StorageError::InvalidInput`** (B9/B9b). A new variant for input an operation refuses to act on. The enum is not `#[non_exhaustive]`; no exhaustive match on it exists in the workspace.
- **`nexus-ai`: forced re-index** (B9c). The `index_file` handler takes an optional boolean `force` (omitted means false; any non-boolean, including `null`, is rejected) that skips only the unchanged-file shortcut. New `rag::index_file_with(.., IndexMode)` with `IndexMode::{IfChanged, Force}`; `rag::index_file` keeps its signature and means `IfChanged`.
- **`nexus-ai`: `EmbeddingProvider::expected_dimension() -> Option<usize>`** (B9c). A defaulted trait method (default `None`), so no implementor has to change. `Some(d)` is a *verified* dimension: OpenAI returns `Some(1536)` (its `embed` always requests `text-embedding-3-small`) and the local provider returns the loaded model's own dimension. Ollama returns `None`: its `dimension()` is a legacy constant for the default model only, the model is configurable, and nothing verifies what the server returns, so a model name is never treated as authority. `dimension()` is unchanged.

### Fixed
- **`nexus-storage`: vector search no longer returns wrong results, panics on NaN or hides damage** (consolidation review batch 3, B9, PR pending). A stale 2-dimension row scored a perfect 1.0 against a 3-dimension query; a stored NaN could panic the result sort; rows that failed to read were dropped silently; a truncated blob decoded to a shorter vector. Similarity is now computed in f64 and returns nothing for mismatched, empty, zero-norm or non-finite vectors, and `search` and note near-duplicate detection exclude those pairs (a threshold of 0.0 no longer reports them). Ties break by path, block id, then row id. A non-finite or empty query is `InvalidInput`. A damaged stored embedding (empty, truncated, non-blob, non-finite) or a file whose chunks have mixed dimensions is `CorruptFile` naming the file, in `search` and `mean_embeddings_by_file`; averaging is summed in f64 and checked finite. **Behaviour change:** a database that already holds such a file now errors until the file is re-indexed (`vector_delete_by_file`, or `index_file` with `force`).
- **`nexus-storage`: vector writes are validated before anything is replaced** (B9b). `upsert` rejects a chunk for another file (it used to insert under the chunk's own path while deleting by the argument path), an empty, non-finite or oversized embedding, mixed dimensions and mixed or empty content hashes with `InvalidInput`; a rejected write leaves the stored rows untouched. `stored_signature` now inspects every row of the file (text hash, one distinct hash, one blob length that is a whole number of `f32`s) and returns none otherwise, so ordinary indexing re-embeds a damaged file.
- **`nexus-ai`: a short or malformed embedding reply no longer truncates silently** (B9c). `index_file` paired chunks with embeddings by `zip`, so a provider returning too few vectors dropped chunks and too many were ignored. The reply is validated (one vector per chunk, non-empty, finite, one dimension, and every vector exactly `expected_dimension()` long when that is `Some`, in normal and forced mode alike) before the storage upsert, and the old vectors are never deleted before a validated replacement. The unchanged-file shortcut now requires a *known* dimension equal to the stored one; an unknown dimension (`None`, Ollama) always re-embeds instead of trusting the legacy constant. The file's previous dimension is not required to match, so a valid model change replaces old vectors; `force` skips only the shortcut, never validation. **Limitation:** a model change that keeps the same dimension is not detected, so with a known dimension an unchanged file stays skipped after such a switch until it is re-indexed with `force`.
- **`nexus-terminal`: `interpolate_env` no longer corrupts non-ASCII values.** `expand_refs` pushed each UTF-8 byte as a Latin-1 `char`, so a value such as `café` came out as `cafÃ©` whether or not it held a `$` reference, and the pass loop compounded the damage. It now decodes whole scalars, the same way `nexus-workflow`'s `substitute_string` already did. Missing-variable, lone-`$`, malformed-reference and cycle behaviour are unchanged and now pinned by tests.
- **`nexus-mcp` builds against `rmcp` 3.1.4 again.** The bump in #473 changed `ServerHandler::call_tool`, `get_prompt` and `read_resource` to return `CallToolResponse`, `GetPromptResponse` and `ReadResourceResponse` (a completed result, a request for client input, or a task); the Nexus handler still returned the bare `*Result` types, so the whole workspace failed to compile in every full CI sweep. The three handlers now wrap their completed results; no behaviour change.

### Changed
- **`nexus-mcp` stops reaching into `rusty-mcp`'s private module and drops dependencies it no longer uses.** The three list methods call the now-public `rusty_mcp::apply_cache_hints` instead of `rusty_mcp::__private::apply_cache_hints`. The direct `http` and `reqwest` dependencies had no remaining uses after the Host client moved into `rusty-mcp`, and `rmcp`'s client and Streamable HTTP client features are now needed only by the in-process adapter tests, so they move to `[dev-dependencies]`. Production builds still get them through `rusty-mcp`'s `client` feature. No behaviour change.
- **The bundled shell is now the workspace's `rush` (`crates/apps/rush`); `nexus-rush` is removed** (RFC 0002). The vendored copy had drifted behind rush. `nexus-terminal` looks for a `rush` binary beside the executable instead of `nexus-rush`, and no longer sets `NEXUS_EMBEDDED_SHELL`: `portable-pty` makes the shell a session leader with the PTY as its controlling terminal, so rush's job control (`fg`, `bg`, Ctrl-Z) works, where nexus-rush had disabled it.

### Fixed
- **Memory sync no longer loses a memory the hub refuses** (`nexus-memory`, design review 2.7 follow-up).
  - `push` ignored the hub's reply and advanced its cursor over the whole page, so a refused memory was never sent again. The sync report also counted it as pushed.
  - A refusal for a timestamp too far ahead of the hub's clock was worse: the cursor jumped to that future time, and every edit made since sat behind it, unsent. A pulled memory from a peer whose clock runs ahead did the same.
  - Refused memories now go to a `sync_push_rejected` dead-letter table in the same transaction that moves the cursor, and every push re-sends them first, as they are then. A memory deleted since is re-sent as its tombstone and kept until the hub takes it; one with no row at all, or no longer authored here, leaves the table.
  - The push cursor never moves past the time the push started. A cursor an older build left in the future restarts from the epoch once; the hub's last-write-wins makes re-sending harmless.
  - The sync report's `pushed` now counts what the hub accepted, and gains `push_refused` and `push_dead_letters`.
- **nexus-rush: `&&`/`||` short-circuit in `$(( ))`, and `[[:class:]]` globs work** (design review 4, shared rush fixtures).
  - `0 && 1 / 0` failed with "division by zero" instead of giving 0, so a guard like `(( n != 0 && total / n > 2 ))` broke when `n` was 0. The skipped side is still parsed, so a syntax error there is still an error.
  - POSIX named classes (`[[:digit:]]`, `[![:alpha:]]`, ...) matched nothing. They are ported from rush, with its bash-verified edge cases.
  - Both were found by the conformance fixtures nexus-rush now shares with rush.
- **Every whole-file write is now crash-atomic and synced** (design review 4, consolidation).
  - Comment sidecars, editor saves and journal, CRDT state, the CLI's CRDT merge driver, the skills registry index, and the shell's persisted state and granted capabilities now share `rusty_atomic_file::write`.
  - Six of these renamed an unsynced temp file, so a power loss could leave an empty or partial file.
  - A temp file left by a crash no longer blocks later saves, since each write uses a unique temp name.
- **A forge switch no longer hands out the old AI runtime** (`nexus-ai-runtime`, design review 4 / N5).
  - The shared pool handle was a `OnceLock`, set by the first forge and never replaced. After shutdown and a new boot in the same process, the indexing daemon got the torn-down runtime.
  - Each pool now replaces the published handle, and clears it on drop if it is still its own. Readers get the live pool's handle or `None` (their existing fallback).
  - `publish_shared_handle` now returns whether it replaced another live pool.
- **An IPC timeout now cancels the handler's token** (`nexus-kernel`, design review 4 / N4). A deadline used to return `Timeout` without signalling the dispatch's cancellation token. Polling handlers, their spawned work, and sync handlers on the blocking pool kept running. A drop guard now cancels the token on timeout or when the caller drops the call, and is disarmed when the handler finishes on its own.
- **Template substitution no longer panics on a short tag** (`nexus-templates`, design review 3.8).
  - `{{ab` sliced five bytes past a four-byte input and panicked; the escape check now uses `starts_with`.
  - Literal non-ASCII text used to be copied byte by byte and came out garbled (`é` became mojibake); it now survives intact.
- **Comment sidecars are written crash-atomically** (`nexus-comments`, design
  review 2.9 / N6). `save` used `fs::write` on the only copy of a file's
  threads, so a crash or short write could leave a truncated sidecar that
  then failed to load. It now writes a synced sibling temp file, renames it
  over the sidecar, and syncs the directory on Unix.
- **Memory hub last-write-wins compares time, not text** (`nexus-memory-hub`,
  design review 2.8 / N3).
  - A pushed `updated_at` is parsed as RFC 3339 and stored as a canonical
    ordering key (UTC, nanoseconds, `Z`), so offsets and precision no longer
    decide which write is newer. The payload keeps the node's own string.
  - A non-RFC 3339 value (e.g. `"zzzz"`, which used to outrank every real
    timestamp and freeze the record) is refused. The push reply now lists
    each refused record's id and reason (`rejected`).
  - Pull cursors are compared as time too, and a malformed one is `400`.
  - On first open, an existing hub's keys are canonicalized, and rows whose
    `updated_at` never parsed move to `records_rejected` (payload kept).
    This is gated by `PRAGMA user_version`, so it runs once.
- **Memory sync no longer loses records it cannot decode** (`nexus-memory`,
  design review 2.7 / N2).
  - The pull loop used to advance its cursor past a whole page but drop
    undecodable records silently, and counted them as transferred.
  - Each page's applied rows, dead-lettered records (new `sync_rejected`
    table: payload, reason) and cursor now commit in one transaction.
  - Every pull first retries the dead letters, so an upgrade that can
    decode them (such as opaque memory ids) recovers them.
  - The sync reply adds `applied`, `unchanged`, `rejected`, `replayed` and
    `dead_letters` beside `pushed`/`pulled`.
- **Importing a real `remind_me` database** (`nexus-memory`, design review
  2.6 / N1).
  - Memory ids are now an opaque `MemoryId`, kept verbatim: Nexus's UUIDs
    and `remind_me`'s `mem_<hex>` alike. `superseded_by` links survive.
  - Previously every `mem_…` row failed UUID parsing and was counted as
    "skipped", so a migration could report success having imported nothing.
  - A row that cannot be imported is now listed in
    `ImportReport::failures`, with its source id and the reason.
  - A re-import is last-write-wins on id instead of aborting on the
    primary key.
  - Stored ids were already `TEXT`, so no migration is needed.

### Added
- **Per-user relay credentials** (`nexus-collab`, gap-analysis §1.4) —
  new `TokenSet`: named tokens with constant-time, full-scan
  verification that returns *which* credential authenticated, so joins
  are attributable in the relay log and one user's token can be
  rotated/revoked without re-keying every peer.
  `RelayServer::new_with_tokens(TokenSet)` alongside the unchanged
  Phase-1 `new(Token)` (now a one-entry set named `default`). TLS
  remains deferred — front the relay with a TLS-terminating proxy.
- **Prometheus exit path for kernel metrics** (BL-093 closure) —
  `MetricsSnapshot::to_prometheus_text()` renders the registry in the
  text exposition format (counters, the queue-depth gauge, and
  p50/p95/p99 summaries in seconds; sorted/deterministic output;
  label escaping per spec), exposed as
  `com.nexus.security::metrics_prometheus` (handler id 10, unrestricted
  read-only) so any frontend or a scrape sidecar can reach it via
  `ipc_call`.
- **Linux + macOS release pipelines** — `release-linux.yml` (`.deb` /
  `.rpm` / `.AppImage`) and `release-macos.yml` (aarch64 + x86_64
  `.dmg`s) mirror the Windows workflow: tag-triggered, artifacts +
  `SHA256SUMS-<platform>-<tag>.txt` checksums attached to one shared
  draft Release, `workflow_dispatch` dry-runs. The Windows workflow
  gains the same checksum sidecar. Auto-updater key-handling steps are
  documented in `RELEASE.md` (owner-generated secrets; no updater code
  yet).
- **Hybrid forge search** (`com.nexus.storage::hybrid_search`, handler id
  76) — reciprocal-rank fusion (`k=60`, matching `nexus-memory`'s recall)
  of the Tantivy BM25 arm and the vector-store cosine arm, with 4×
  per-arm oversampling so blocks outside one arm's window can still win
  on fused rank. The caller supplies query text + embedding (storage
  does not embed — D-1). Reachable end-to-end via
  `com.nexus.ai::semantic_search` with `"hybrid": true`; either arm
  degrades gracefully when empty.
- **Cognitive-store persistence (memory Phase 5)** — `MemoryStore` (the
  episodic / semantic / procedural facade in `nexus-memory`) gains optional
  SQLite write-through: `MemoryStore::open(forge_root)` loads prior state
  from `.forge/memory/memory.db` (new `episodic_log` / `semantic_facts` /
  `procedural_skills` tables alongside the plugin's `memories` table) and
  persists every subsequent mutation, so agent memory survives process
  restarts. `MemoryStore::new()` keeps the original in-memory semantics; the
  API surface is unchanged, exactly as the Phase-1 docs promised.
- **Common plugin contract + conformance gates** (#187 / V9) —
  `@nexus/extension-api`'s `NexusPluginContext` is re-derived as the subset
  both live runtimes satisfy (with `MaybePromise` bridging sync/async);
  compile-only conformance tests lock the in-process `PluginAPI` (which now
  carries a host-asserted `pluginId`) and the sandbox
  `SandboxedPluginContext` to it. `ScriptPlugin` is deprecated (removal
  0.2.0) and the package re-cut `1.0.0` → `0.1.0`.
- **Native memory engine at full `remind_me` parity** (`com.nexus.memory`,
  #188) — promoted from a staging library to a wired service plugin with 21
  IPC handlers: CRUD/list/stats, FTS5 + hybrid-vector recall (RRF), SPO facts
  + entity graph, tags, ACT-R vitality, `auto_capture`/`get_capture`/
  `consolidate`, LLM `wiki_*` synthesis, `export`, and cross-instance `sync`
  against the new standalone `nexus-memory-hub` server. Plus passive event-bus
  capture. Reachable from CLI, TUI, MCP (`nexus_memory_*`), and the shell
  Memory Dashboard. See [`docs/0.1.2/memory.md`](docs/0.1.2/memory.md).
- **OS process sandbox** (Phase 4 F1/F2) — a Codex-style `SandboxPolicy`
  (`read-only` / `workspace-write` / `danger-full-access`) in `nexus-types`,
  enforced on Linux via Landlock (filesystem) + seccomp-bpf (network
  off-by-default), composed by `confine_current_thread`, and applied to
  spawned children through the single-threaded `nexus-sandbox` helper.
  Permissioned download broker for approved egress under a network-off policy.
  Configured via `.forge/sandbox.toml`, reachable over IPC
  (`com.nexus.security::sandbox_policy` / `download`), MCP, the `nexus sandbox`
  CLI, and a shell panel; opt-in per terminal session. See
  [`docs/0.1.2/os-sandbox.md`](docs/0.1.2/os-sandbox.md).
- `security.audit.read` capability gating `query_audit_log` (previously
  unrestricted; cross-plugin telemetry is reconnaissance surface).
- `cargo-deny` supply-chain gate in CI (`deny.toml`): advisories,
  license allowlist, duplicate bans, registry provenance.
- One-shot operator warning when a remote AI provider is configured with
  credentials but without TLS pinning.
- Tauri command-boundary guard now runs on every PR
  (`crates/nexus-bootstrap/tests/tauri_command_boundary.rs`).
- 22 characterization tests over linkpreview's OG/Twitter-card parsing.

### Changed
- Outbound HTTP clients carry timeouts: 10s connect + 300s read backstop
  for AI providers, 10s/30s for notification webhooks.
- Storage knowledge-graph reads recover from lock poison instead of
  aborting the process (`panic=abort`) — #199 tier-1 policy.
- Linkpreview pins each fetch hop to its SSRF-validated IP, closing the
  DNS-rebinding TOCTOU.
- `scripts/` reduced to the five portable value-add helpers; the
  single-machine cargo wrappers were removed.
- Shell chrome no longer imports workspace-plugin internals: new
  `WorkspaceHostSurface` seam (plugin registers at activation), with the
  host→plugin import direction now test-enforced.
- Shell test stubs are structurally type-checked (`stubPluginAPI`);
  zero `as any` remain in shell test files.
- Kernel `context_impl.rs` split into focused modules (pure code motion).

### Security
- See Added/Changed: audit-log read gating, supply-chain CI gate,
  DNS-rebinding fix, HTTP timeouts. Advisory RUSTSEC-2025-0068
  (`serde_yml`, unsound/unmaintained) is acknowledged and tracked in #248.

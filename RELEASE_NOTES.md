# Release Notes

Tracks the **monorepo itself** — crate merges, workspace-wide CI, and
cross-crate changes (like the duplication sweeps below) — not each crate's
own internal changes, which are logged in that crate's own
`crates/<name>/RELEASE_NOTES.md` where one exists (many crates kept theirs
from before the merge; see ADR-0001 for why root and per-crate logs are
separate rather than one superseding the other).

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. Bolded inline category tags (`**Added:**` / `**Changed:**` /
`**Fixed:**`), known limitations stated plainly.

---

## 2026-10-08 - RLEvalSystem imported into crates/apps/rocket_league (pending review)

- **Added:** `replay-analyzer`, `replay-scoring`, `replay-skills`, `replay-value`, `replay-viewer`, `replay-pacifist`, `bc-clone`, `recon-check`, `rleval-app` and the `rleval/` product dir (docs, Python `service/`, scripts). History preserved via `git filter-repo` (201 of 222 commits; the rest only touched private material). Stacked on the rusty_bullet import.
- **Changed:** replays, corpus, player sessions and the Spire capture kit are excluded and live in the private repo `baileyrd/rocket_league_private`. `rleval-app`'s git-pinned rusty_mill dependencies became workspace dependencies. `replay-analyzer`'s golden records `boxcars-0.11.5` (was 0.11.3; no other change). `rleval-app`'s SHA-256 uses `as_chunks`, `rp-core`'s `refills_over_time` test uses a 600/min bucket (it failed 43 of 96 runs under CPU contention before, 0 of 96 after; it started failing in CI once this import changed the test sharding), and its upload-signing key comes from `rusty_rand` (the old `/dev/urandom` read made signed uploads return 501 on Windows, which the old Ubuntu-only CI never ran). `history_flow` skips without the corpus.
- **Known limitation:** the Python service and Docker files are moved, not built or tested in CI; the viewer headless-GL smoke test and the `mmdb,oidc` feature job of the old CI are not ported yet.

---

## 2026-10-08 - rusty_bullet imported as crates/apps/rocket_league (pending review)

- **Added:** `crates/apps/rocket_league/` (ADR-0008) with `rb_domain`, `rb_env`, `rb_physics_bullet`, `rb_replay_ingest`, `rb_capture_ingest`, `rb_scenario`, `rb_verify_cli` and the product's docs, BakkesMod plugin and tape-bot tooling. Full history preserved via `git filter-repo` + merge (SHAs differ from `baileyrd/rusty_bullet`; map in the PR).
- **Changed:** `Cargo.lock` gains 20 packages, no existing entry changes. `boxcars` is held on one 0.11.x so `subtr-actor ~1.2` and `rb_replay_ingest` share a `Replay` type.
- **Known limitation:** `tools/rb_tape_bot` is excluded from the workspace (own lockfile, external `rlbot` client). RLEvalSystem is a separate follow-up import; replays and corpora stay out of git.

---

## 2026-10-08 - rusty_bbp: Blackboard Protocol core joins libs/protocol (pending review)

- **Added:** `rusty_bbp` at `crates/libs/protocol/rusty_bbp`, Tier S (`rusty_serde`, `rusty_rsa`; `proptest` dev-only). Moved from `baileyrd/rusty_bbp` after its stage-1 tests passed, so its first-party dependencies are path dependencies under the workspace layer check instead of a git pin. No consumers yet; `rusty_orch` is the intended first one.
- **Known limitation:** stage 1 only. The store is in-memory; the durable log, the MCP adapter and the runner supervisor are stages 2 and 3. Tokens and run secrets are deterministic hashes until `rusty_rand` is wired in at stage 3.

---

## 2026-10-08 - rusty-mcp-client split (pending review)

- **Added:** `rusty-mcp-client`; **Changed:** `rusty-mcp` loses its `client` feature; `rk-mcp` depends on the new crate. Stdio verified again against a real `rusty-mcp-demo` after the move. No OpenSSL in the MCP client path (the `native-tls` still in `rk-mcp`'s tree comes from `aisdk`/`rk-feed`, the LLM client).
- **Known limitation:** Nexus is frozen and still references `rusty_mcp::client`. The Streamable HTTP path is still not exercised against a live server.

---

## 2026-10-08 - rk-mcp client on rusty-mcp (pending review)

- **Changed:** `rk-mcp`'s stdio and HTTP adapters collapse into `RemoteMcpClient` over `rusty-mcp`'s client. Verified end to end for stdio: the ignored smoke test connected to a real `rusty-mcp-demo`, listed tools, reconnected and listed again.
- **Known limitation:** the Streamable HTTP path was not exercised (no server to point at here), and its TLS backend changes to `native-tls`. A server with more than one page of tools now returns all of them, so a registry that relied on the truncated list will see more tools.

---

## 2026-10-07 - rk-app MCP server on rusty-mcp (pending review)

- **Changed:** `rusty-keys --mcp` uses `rusty_mcp::serve`; compiles clean under clippy `-D warnings` with `--features mcp-server`, and the crate's tests pass.
- **Known limitation:** the MCP path is still not exercised end to end (needs a live model; the file header already says so), so the protocol-version change and shutdown behaviour are compile-checked only. Try `rusty-keys --mcp` from an MCP client before relying on it.

---

## 2026-10-07 - rusty_dirs (pending review)

- **Added:** `rusty_dirs`; `rusty_term` and `rusty-croc` use it.
- **Known limitation:** only config directories are shared. The other ~6 sites in the audit (`rusty_yirp` state dirs, `rusty_inventory`, `rusty_key` guide, `rusty_provider` `~` expansion, `rush`, `sessionmgr-agents`) resolve different things (state/data dirs, `~` expansion, override variables) and were left alone; `rusty_yirp`'s two path files are duplicated on purpose. Nexus untouched (frozen, deletion not approved).

---

## 2026-10-07 - rmcp pin and rsi blob writes (pending review)

- **Changed:** `remind_me_remote` uses the workspace `rmcp`; `rsi-runtime` blob writes are now fsynced (slightly slower, durable).
- **Known limitation:** `rusty_fair_play`'s `write_by_rename` and `rusty_lines`' history writer were not moved to `rusty_atomic_file`. Fair Play's tests make a directory at the fixed `<file>.tmp` name to force a write failure, which `rusty_atomic_file`'s unique temp names defeat; `rusty_lines` preserves an existing file's permissions, which `rusty_atomic_file` does not.

---

## 2026-10-07 - Retry-After and backoff consolidated (pending review)

- **Changed:** five crates use `rusty_retry` for backoff or `Retry-After`; `rusty_http` gains a `rusty_retry` dependency (it re-exports the date parser).
- **Known limitation:** the gateway's `initial_backoff` larger than `max_backoff` now sleeps `max_backoff` from the first attempt (it slept the initial value once before). Nexus's four retry copies are untouched (frozen).

---

## 2026-10-07 - rusty_percent (pending review)

- **Added:** `rusty_percent` foundation crate; **Changed:** eleven crates drop their private percent-encode/decode helpers for it. Fixes trailing-escape decoding in `remind_me_core`, `remind_me_hub`, `remind_me_api` and `rusty_fedora_agent`.
- **Known limitation:** `rusty_url` keeps its own spec-driven `percent_decode`; `remind_me_hub`'s test helper `urlencode` and eight crates that use the external `percent-encoding` crate (`rusty-mcp` trace, `rp-providers`, `rusty-search-*`, meshed) are untouched. Follow-up, not part of this change.

---

## 2026-10-07 - rusty_hex (pending review)

- **Added:** `rusty_hex` foundation crate; **Changed:** eight crates drop their private hex codecs for it. `ts-key` and `ts-types` no longer each carry a copy.
- **Known limitation:** `rusty_term`'s `gui` feature and `platform-bsd` were not built here. `rusty_term` clippy fails without gui on unused `Grid` methods, also on the untouched baseline.

---

## 2026-10-07 - base64 copies replaced (pending review)

- **Changed:** six hand-written base64 encoders/decoders now call `rusty_base64`; new `decode_standard_lenient` (+2 tests) covers PEM and terminal payloads.
- **Known limitation:** `platform-bsd`'s edit compiles only on BSD targets and was not built here; it is the same one-line wrapper as `platform-linux`'s. `rusty_term`'s `gui` feature (encode alias) was not built. Pre-existing: `rusty_term` clippy `-D warnings` fails on unused `Grid::running_command`/`abs_line_text`/`row_text` without the gui feature, also on the untouched baseline.

---

## 2026-10-07 - One strip_ansi (pending review)

- **Changed:** `rusty_ansi` parses two-byte escapes and string sequences (new `AnsiToken::Escape`); `rp-router` and `rusty_lines`' PTY tests use `rusty_ansi::strip_ansi` instead of private copies. Five new `rusty_ansi` tests; all 43 `rusty_lines` PTY tests pass.
- **Known limitation:** an unterminated string sequence (`ESC _ ...` with no `ST`) still leaks its text; only the `ESC` is dropped. The Nexus copies are untouched (frozen).

---

## 2026-10-07 - Emitter helpers for AG-UI adapters (pending review)

- **Added:** `Emitter::{text_delta, end_text, tool_call, tool_result}` in `rusty_agui`; **Changed:** `adk-agui`, `rk-agui`, `rusty_tick` and the `echo_agent` example use them instead of private copies.
- **Known limitation:** an adapter must still call `end_text` before a non-text event (the verifier rejects a tool call inside an open message). The helpers do not close it for you.

---

## 2026-10-07 - One Bearer parser (pending review)

- **Changed:** `rusty_oauth::bearer::token_from_authorization` replaces nine call-site copies. Behaviour change: scheme matched case-insensitively and an empty token is rejected up front. New `rusty_oauth` edges: `remind_me_core`, `remind_me_api`, `remind_me_remote`, `rp-server`, `rk-app`, `rusty_fair_play`, `rusty_tick`.
- **Known limitation:** `remind_me_api`'s and `remind_me_remote`'s flat-key check still compares the whole header to `"Bearer <secret>"` in constant time (exact case); only the scoped-key and OAuth paths use the parser.

---

## 2026-10-07 - One constant_time_eq (pending review)

- **Changed:** `rusty_crypto_key::constant_time_eq` replaces seven hand-written copies and the one inside `SecretBytes::eq`; the length is folded into the accumulator and the shorter side zero-padded, so a wrong-length token no longer returns early. New edges: `rusty_oauth`, `remind_me_core`, `remind_me_hub`, `rk-app`, `rusty_fair_play`, `rusty_tick` depend on `rusty_crypto_key`.
- **Known limitation:** the two Nexus copies (`nexus-collab`, `nexus-memory-hub`) are untouched because Nexus is frozen. `rk-app`'s old `&str` unit test moved to `rusty_crypto_key`.

---

## 2026-10-07 - Nexus frozen (pending review)

- **Changed:** the 41 `nexus-*` crates are excluded from the workspace (`exclude` in the root `Cargo.toml`), so CI, `--workspace` runs, the workspace map and `Cargo.lock` no longer cover them. `Cargo.lock` only loses packages (162 removed, 41 Nexus); none added or upgraded.
- **Known limitation:** frozen crates do not build until unfrozen (`crates/apps/nexus/FROZEN.md`). `NEXUS_NO_KEYRING` and the D-Bus setup in CI are left in place, now unused. Dependabot's config never listed Nexus. `docs/WORKSPACE-ATLAS.html` was not regenerated.

---

## 2026-10-07 - Term and Tick Tailwind 4 CI repair (pending review)

- **Fixed:** both web builds use `@tailwindcss/vite` 4.3.3, import Tailwind's v4 CSS, and explicitly load their existing theme configurations. Existing dependency versions, React integration, aliases, chunking, proxies, and CI gates are unchanged.
- **Fixed:** renamed outline, small shadow, and small radius utilities retain their intended appearance; base compatibility rules retain unspecified border/placeholder colors and button cursors.
- **Added:** production Chromium regressions verify Term's three theme presets and opacity, plus Tick's light/dark colors, opacity, focus rings, forced-colors outlines, popovers, and editor styles. Term's preview launcher now converts its file URL correctly on Windows.
- **Validation:** Node 22 builds, Tick's 645 unit and 20 backend integration tests, and both browser suites passed locally. Term's existing immediate-count pane-cap assertion failed on one earlier run and passed unchanged on rerun; Linux CI and the separate Term live-bridge suite have not been run.

---

## Consolidation review batch 3: rusty_meshed clock migration (BREAKING)
**2026-10-06** · PR pending · consolidation review, rusty_meshed clock helpers

- **Changed (breaking):** every wall-clock read in rusty_meshed outside the entropy seed and one stored `SystemTime` now goes through `rusty-meshed-core`'s `ClockReading`, `Timestamp` and `WallClock`; the eight private `now_iso` formatters and the `now_millis` helpers are gone. A pre-1970 or out-of-range clock is an error, not 1970-01-01 or a wrapped integer. The migration guide (old call to new call, including the removed constructors and the new `Clock` error variants) is in the root `CHANGELOG.md` entry of the same name.
- **Fixed:** the relay reads the clock once per non-empty batch before the first produce, so a clock failure sends nothing and the batch shares one `published_at` and record time. Each publisher (`DataProductProducerBase`, `PersonnelLifecycleProducer`, `SLOViolationPublisher`) and each consumer/producer `startup` reads it before any I/O.
- **Fixed:** freshness and completeness no longer say "has never published" for a broker error, a failed request or an unreadable clock; they say "age unavailable: <reason>". An empty partition still says "never published". `actual_value` stays infinite and the lag arithmetic is bit-for-bit unchanged.
- **Added:** the outbox relay thread logs its errors to stderr with rate limiting and a recovery line, and no new dependency.
- **Known limitations:** the access-grant router handler and the CLI's best-effort violation publish read the system clock directly, so their clock-failure paths (HTTP 500, skipped publish) are not covered by an injected-clock test. The two transformation handlers read the clock before the database is opened or seeded and are covered (an unreadable clock leaves a fresh database empty). Consumers outside this repository that depend on the removed signatures by git or path will not compile. Not run: the Windows matrix and a full workspace check.

---

## Consolidation review batch 3: nexus vector integrity
**2026-10-06** · PR pending · consolidation review B9, B9b, B9c

- **Fixed:** `nexus-storage` vector search. A dimension-mismatched row scored 1.0 against a 3-d query; NaN scores could panic the sort; unreadable rows were dropped silently; truncated blobs decoded as shorter vectors. Similarity is now f64, checked, and undefined pairs are excluded (also from note near-duplicate detection at threshold 0.0); damaged embeddings and mixed-dimension files are `CorruptFile` errors naming the file; ties break by path, block id, row id.
- **Fixed:** `upsert` is validated before anything is replaced (wrong-file chunk, empty or non-finite or oversized embedding, mixed dimensions or hashes); `stored_signature` inspects every row so ordinary indexing re-embeds a damaged file; `index_file` rejects a short, long or malformed provider reply instead of truncating.
- **Fixed:** a provider reply of the wrong size no longer replaces good vectors. New defaulted `EmbeddingProvider::expected_dimension() -> Option<usize>` (`None` by default; OpenAI `Some(1536)`, local its model's dimension, Ollama `None`): a `Some` dimension rejects every reply vector of another length in normal and forced mode, and the unchanged-file shortcut needs a known dimension equal to the stored one, so an unknown dimension always re-embeds. A valid dimension change still replaces old vectors. **Limitation:** a model change that keeps the same dimension is not detected.
- **Added:** `StorageError::InvalidInput`; `index_file` `force` (optional boolean, malformed values rejected); `rag::index_file_with` and `IndexMode`.
- **Behaviour change:** a database that already holds a mixed-dimension or damaged file now fails `search`, hybrid search and averaging until that file is re-indexed (`vector_delete_by_file` then `index_file`, or `index_file` with `force`). Hybrid search also fails on an invalid query vector.
- **Known limitations:** a non-finite float inside a correctly sized blob is invisible to `stored_signature` (new writes cannot create one; `force` repairs old ones). `rp-core::RateLimiter` FIFO, the embedder-availability race, the `rusty_err` source chain and the rusty_meshed clock migration are not part of this batch. Not run here: the Windows matrix and the full workspace check.

---

## rusty_tick on CopilotKit: a React page and runtime
**2026-10-06** · [#534](https://github.com/Rusty-Mill/rusty_mill/pull/534) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · `crates/apps/rusty_tick/copilotkit-demo`

- **Added:** a Node project with CopilotKit 1.77's runtime (`runtime.mjs`: `CopilotRuntime` with `rusty_tick`'s `POST /api/agent` as an `@ag-ui/client` `HttpAgent`, bearer token held server side, loopback only) and a Vite React page (`CopilotChat`, a `create_task` tool through `useFrontendTool`), and two runtime routes, `GET` and `POST /api/demo/tasks`, that read and add tasks in `rusty_tick`'s Inbox (title validated, body capped, `502` on a `rusty_tick` failure). The page lists the Inbox, so tasks persist and reloads show them. Typing `add buy milk` streams the assistant's text, runs the tool in the page, and shows the assistant's confirmation on the follow-up run.
- **Added:** nothing leaves the machine: `COPILOTKIT_TELEMETRY_DISABLED` is set by the runtime, the page sets `enableInspector={false}` (the inspector otherwise fetches Google Fonts and `cdn.copilotkit.ai/notifications`), and `scarfSettings` disables install-time analytics. The browser's network panel shows only the page's own origin.
- **Verified:** `tsc --noEmit`, `vite build`, and a headless Chromium run against a real `rusty_tick`, the runtime and Vite: the task appeared and the assistant confirmed. This is the first check of the AG-UI endpoint against CopilotKit's own runtime and React SDK; before it, only the reference client had driven it.
- **Known limitations:** run by hand, not in CI; pins CopilotKit `1.77.0` and `@ag-ui/client` `1.0.2`; `rusty_tick`'s agent understands only `add <title>` and does not read the task list; only the Inbox is shown.

---

## rusty_tick: fix assignee/estimate save clash and calendar view menu order
**2026-10-06** · [#530](https://github.com/Rusty-Mill/rusty_mill/pull/530)

- **Fixed:** a task could not have both an assignee and a pomo estimate: both docs used the task id as their document id, the server keeps one document per id whatever its kind, and the refused second save (409) was hidden by the optimistic UI, so it was gone on reload. Each kind now has its own derived id (`derivedId` in `lib/id.ts`, the hash `checkinId` already used, with check-in ids unchanged). A doc saved under the bare task id by #520 or #525 is removed when that task's assignee or estimate next changes.
- **Fixed:** the calendar view menu listed "3 Days" above "Month" (integer-like object keys sort first); it now follows `CALENDAR_MODES`.
- **Verified:** web `tsc --noEmit`, `vitest` (645, with regression tests for both), `npm run build`; the new features run in Chromium against the real server with no console errors. Rust unchanged.
- Known limitations: the web app's background save takes about 20 s for a 317-event import (the on-screen count appears after about 2 s); tasks still queued when a tab closes are sent on the next open; subscriptions are not recreated for tasks whose queued creation was lost.

---

## Consolidation review batch 2: whisper log sink, nexus env UTF-8, Provider FIFO, search Value bridge, rusty_time from_unix_secs
**2026-10-06** · [#527](https://github.com/Rusty-Mill/rusty_mill/pull/527) · consolidation review B4, B5, B6, B7, B8 part 1

- **Fixed:** `rusty_whisper::log` runs the installed sink with no lock held and drops a replaced sink after the lock is released; poisoned locks are recovered. A reentrant sink, a sink whose captured values log on drop, and a panicking sink are covered by child-process tests.
- **Fixed:** `nexus-terminal::interpolate_env` decodes whole UTF-8 scalars instead of pushing bytes as Latin-1 chars; `café ${X}` interpolates to `café ok`. Malformed references and cycles behave as before, now pinned.
- **Changed:** `rp-router` caches share one crate-private `fifo::FifoMap`; `ReasoningReplayCache` gains an eviction test. `RateLimiter` in `rp-core` deferred.
- **Added:** `rusty-search-core::serde_json_bridge` behind default-off `serde-json` (optional `serde_json`); six backend copies removed; lossy points tested. Verified with the feature off (no `serde_json` in the core's dependency tree), on, per backend, and with a joint check of the twelve crates that depend on the core; a full workspace check was blocked in the build container by a missing `libdbus` system library and is left to hosted CI.
- **Added:** `rusty_time::DateTime::from_unix_secs(i64) -> Result`, checked against the `i32` year range, round-trip tested across 1900–2200 plus the `i64` and `i32`-year extremes. The meshed clock-helper migration is held: its proposed epoch fallback would have recorded a false timestamp on conversion failure, and is being redesigned around explicit error propagation.
- **Known limitations:** eight equivalent `now_iso` copies remain in rusty_meshed; `rp-core::RateLimiter` still carries its own FIFO block.

---

## rk-agui: a Rusty Keys session over AG-UI
**2026-10-06** · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 10, `rusty_key`; step 10 complete

- **Added:** `rk-agui` at `crates/apps/rusty_key/crates/agui`: `KeyAgent::new(handle, config, model)` implements `rusty_agui::serve::Agent`, so `AgentHandler` on `rusty_serve` serves Rusty Keys to everything that speaks AG-UI. A `Session` is built per thread on first contact, the way `rk_app::acp` builds one per editor session, with an `ApprovalGate` at the end of its policy chain; `with_approval(triggers)` says which tool calls the gate stops.
- **Added:** the mapping, by the harness's own `rk://` names. The last user message is the turn's prompt and the session keeps the transcript; `token` streams as deltas of one message, or the reply goes out as one message when nothing streamed; each `tool_event` is `TOOL_CALL_START`/`ARGS`/`END` plus a `TOOL_CALL_RESULT` carrying the payload, or `{status, payload}` when the outcome was not `ok` (a blocked call is visible as such); `turn_complete` is the run's result, the boundary `TurnResult` (`reply`, `verified`, `limits`); a turn error, a missing or blank prompt, a prompt while a turn waits on an approval, and a tool message answering nothing pending are `RUN_ERROR`. `bash_output`, `entropy` and `consolidation` are not forwarded.
- **Added:** human in the loop, twice. An `approval_request` ends the run with a call to the frontend tool `approve_tool` (tool, arguments, trigger) while the turn waits in the gate; the tool message that answers it on the next run (`allow`, `always`, anything else blocks) is delivered to the gate and the same turn is relayed on. A `plan_exit` ends the run with a call to `plan_decide` carrying the plan; the answer (`proceed`, `reject`, `annotate <feedback>`) resolves it and the feedback is the run's result for the client to send as the next turn.
- **Tests:** five end-to-end tests with `rusty_agui`'s client over a socket against the real harness (registry, workspace policy, aisdk loop, verifier, journal) in a temporary workspace with the scripted `FakeLanguageModel`: a turn that reads a file and replies; an approval answered with `allow` (the file is written), a prompt and a stray answer refused meanwhile; an approval blocked (no file, a blocked tool result, the turn continues); a plan exit annotated and resolved; missing and blank prompts. Two unit tests for the answer parsers.
- **Known limitations:** blocking over a tokio handle; `bash_output` is not streamed; sessions live in memory for the process; a `Session` per thread shares the workspace's `.rustykeys` state, as the gateway's multi mode does.

---

## adk-agui: a Rust ADK agent over AG-UI
**2026-10-06** · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 10, `rusty_adk`

- **Added:** `adk-agui` at `crates/libs/rusty_adk/crates/adk-agui`, the AG-UI counterpart to `adk-a2a`: `AdkAgent::new(runner, handle)` implements `rusty_agui::serve::Agent`, so `AgentHandler` on `rusty_serve` serves any ADK agent or graph to everything in the workspace that speaks AG-UI. `with_user_resolver` maps a run to an ADK user (default: `forwardedProps.userId`, else the thread id); `with_run_config` applies one `RunConfig` to every run, streaming included.
- **Added:** the mapping. Thread = session, created on first contact; the last user message is the new turn and the client's history is not replayed (ADK keeps its own); a model text part is a text message, streamed as deltas under one message when the run config streams and the aggregated event closes it without repeating the text; `FunctionCall` and `FunctionResponse` parts are `TOOL_CALL_START`/`ARGS`/`END` and `TOOL_CALL_RESULT`; a run that changed non-`temp:` state ends with a `STATE_SNAPSHOT` of the session; an event with an error, a failed stream, a run without a user message or a tool message answering no pending interrupt is `RUN_ERROR`. Artifacts and thoughts are not forwarded.
- **Added:** human in the loop. A graph node's `resume_or_request_input` ends the run with a call to the frontend tool `request_input` (id = the interrupt id, arguments = the node's `hint` and `payload`); the tool message that answers it, on the next run of the thread, resumes the graph at that node with the answer (JSON when it parses, text otherwise) as the payload. A React, Vue or Angular client registers an action named `request_input` with a `render` and the person answers through `respond`.
- **Added:** feature `agui` on `rusty-adk` (`rusty_adk::agui`), and the `agui-agent-server` example: the same approval graph as `a2a-agent-server`, served at `http://127.0.0.1:8080/api/agent`, with the two `curl` calls that suspend and resume it.
- **Tests:** five end-to-end tests with `rusty_agui`'s blocking client against the handler over a socket (a tool-calling agent with its text, calls, result and state; two turns on one thread landing in one session; a streamed run whose deltas arrive once; a suspension answered and resumed, and a stray answer refused; a run with no user message), plus two unit tests for the value bridge.
- **Known limitations:** blocking only (`Handle::block_on` from the handler's thread; a current-thread runtime that something else blocks would deadlock); no artifacts; one agent serves one run at a time, as `AgentHandler` does; the `nexus-ai-runtime` and `rusty_key` adapters are still to come.

---

## @rusty-mill/agui-vue and @rusty-mill/agui-angular: the Vue and Angular bindings
**2026-10-06** · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 8

- **Added:** `@rusty-mill/agui-vue` at `crates/libs/protocol/rusty_agui/packages/agui-vue`: `provideAgent(config)` in a `setup` (or `app.provide(AGENT, new AgentStore(config))` for the whole app), `useAgent()` with computed refs for `messages`, `state`, `running`, `error` and `toolCalls` plus `send`, `run`, `stop` and `renderToolCall`, `useReadable` and `useAction` that follow a ref or getter and unregister when the scope ends, and `useSharedState()` as a writable computed ref. Peer dependency `vue ^3.5`.
- **Added:** `@rusty-mill/agui-angular` at `crates/libs/protocol/rusty_agui/packages/agui-angular`: `provideAgent(config)` as providers for a component, a route or the application, `injectAgent()` with signals for the same five fields and the same verbs, `injectReadable` and `injectAction` that follow a signal through `effect` and unregister with the injection context, and `injectSharedState()` as `[signal, setState]`. Peer dependency `@angular/core >=19`.
- **Changed:** `AgentStore` and its types (`Snapshot`, `ActionDefinition`, `ActionRenderProps`, `StoreConfig`, `SendOptions`, `ToolCallStatus`, the new `ToolCallEntry`) move from `@rusty-mill/agui-react` into `@rusty-mill/agui-core`'s `store` module, where the three bindings share them; `renderToolCall` and `parseArguments` move into the store too. The React package re-exports them, so its API is unchanged. The scripted fake agent the bindings' tests use is exported as `@rusty-mill/agui-core/testing`.
- **Tests:** six store tests in the core (streaming and `RUN_ERROR`, readables and state both ways, a handler answered and followed up, a render-only call answered through `respond` and a second answer ignored, invalid arguments and `followUp: false`, lenient argument parsing); five Vue tests mounting components with `createApp` under jsdom; five Angular tests on a zoneless `createApplication` whose `tick()` runs the root effects. Each binding passes the same scenarios the React one does, plus a changed readable or action definition replacing its registration and a destroyed scope ending the subscription.
- **Known limitations:** no consumer in the workspace yet for either binding (the follow-ons recorded step 8 as waiting for one; it was started on request); no components; Angular's `render` returns whatever the template hands a child, the binding does not render.

---

## rusty_bot: per-bot sandboxes
**2026-10-06** · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 7, second PR

- **Added:** `rusty_bot` at `crates/libs/rusty_bot`. `BotSpec::sandbox()` builds the `SandboxSpec` for one agent process: write only to its workspace (also its working directory), read its roots (by default `/usr`, `/lib`, `/lib64`, `/bin`, `/etc`) and the directory its program lives in, exactly the environment it was given, and `default_limits()` (a day of CPU, a week of wall clock, 2 GiB of address space, 1 GiB files, 1024 descriptors, 256 processes). The network stays open (`Sockets::Internet`): a bot listens on a port and may call a model; the filesystem is what keeps one bot's data from another's, the same posture `rsi` takes for coding-agent CLIs. `start` runs a bot with a thread waiting on it; `Running::stop` kills the process group and `outcome`/`wait` report how it ended; `Fleet::start` is all up or all down. `load` reads a JSON bots file and refuses duplicate names.
- **Added:** the `rusty-bot` binary: `run <bots.json> <state dir>` starts the fleet and prints each bot's end; `__sandbox` is the helper the executor spawns, single-threaded from birth. Having both in one binary is how `rsi` does it, and it means the helper is always the same build as its caller.
- **Changed:** `rusty_sandbox::ProcessExecutor::start` returns a running `Job` instead of waiting: `Job::handle()` is a `Copy` `JobHandle` whose `kill()` sends `SIGKILL` to the group from any thread, and `Job::wait(wall)` reaps and contains it as `exec` always did (`exec_with` now runs on the same path). `start` waits for the helper to either fail setup or replace its image before reading the status file, checked through `/proc/<pid>/exe`: a helper that has not opened the file yet must not find it gone.
- **Tests:** two unit tests (the sandbox a spec builds; loading with defaults and five bad documents); two Linux integration tests through the real binary (a shell bot writes `note.txt` in its workspace, cannot create a file outside it, and dies as a group with `SIGKILL` when stopped; a fleet with one bad spec starts nothing, a good one starts and stops); one more executor test (a started job killed from its handle). Off Linux, the test asserts the refusal.
- **Known limitations:** no port allocation or health check (a bot's port is whatever its own arguments say); no restart policy; the fleet is in memory; wiring a bot's port to a channel runner or the gateway is the operator's, by `AGENT_URL`.

---

## rusty_sandbox: the sandboxed executor, hoisted from rusty_rsi
**2026-10-06** · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 7, first PR · amends [ADR-0005](docs/adr/0005-rsi-harness.md) §4

- **Added:** `rusty_sandbox` at `crates/libs/rusty_sandbox`: the sandboxed-execution port (`SandboxSpec`, `Limits`, `Executor`, `ExecOutcome`, `Termination`) and its Linux adapter (`ProcessExecutor`, the helper's `run_helper`, `HelperRequest`, `Sockets`, `SETUP_FAILED`, `require_enforced`), moved from `rsi-core` and `rsi-runtime` with their fifteen tests. The code is unchanged but for its error type: `rusty_sandbox::Error` has `Io`, `Sandbox` and `Invalid` (a rejected limit or path), where the port used `CoreError::InvalidParameter`/`InvalidId` and the adapter `RuntimeError`.
- **Changed:** `rsi-core::exec` re-exports the port; `rsi_runtime::executor` and `rsi_runtime::sandbox` re-export the adapter, so `rsi __sandbox`, the harness, the graders, the coding-agent runner and every test keep their paths. `CoreError` and `RuntimeError` gain `From<rusty_sandbox::Error>` (a rejected spec is an `InvalidId`; I/O and setup failures keep their runtime variants), and `rsi-runtime`'s grading impls take any `Executor` whose error converts rather than one whose error *is* `RuntimeError`. `rsi-runtime` keeps `rusty_libc` for its own file flags; `platform` and `platform-linux` moved with the helper.
- **Why now:** step 7's second PR gives each bot a sandbox of its own; that needs the executor in `libs`, where an app family other than `rsi` can depend on it (ADR-0003 forbids `apps → apps`). ADR-0005 scoped the executor to `rsi`; this amends it with the user's sign-off.
- **Known limitations:** unchanged from `rsi`: Linux only, and the helper must be a single-threaded binary; the next PR adds one for bots.

---

## rusty_routine: routines for AG-UI agents
**2026-10-05** · [#524](https://github.com/Rusty-Mill/rusty_mill/pull/524) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 6

- **Added:** `rusty_routine` at `crates/libs/protocol/rusty_routine`. `Schedule` is five-field cron in UTC with `*`, numbers, ranges, lists and steps; `0` and `7` are Sunday; when both day fields are restricted either matches, as in cron; `next_after(t)` finds the first scheduled minute strictly after `t` by Hinnant's civil-date arithmetic and gives up after five years for a date that never comes. `Routine::fire(now)` returns the run (a fresh thread per firing, the prompt as the person's message, `forwardedProps.routine` and `scheduledAt` for gateway rules) and advances the schedule past `now`, so a firing the runner slept through is skipped rather than made up. `Routine::record` counts consecutive failures and disables the routine at its budget; a success resets it. `load` reads a JSON array of `{"name", "cron", "prompt", "maxFailures"?}` and refuses duplicate names.
- **Added:** behind the `run` feature, `run::tick` fires every due routine against one `HttpAgent`, reads the reply back through `rusty_channel::Thread`, and reports one line per run; `run::run` sleeps until the earliest next firing and stops once every routine is disabled. The `routines` example reads `ROUTINES`, `AGENT_URL` and an optional `AGENT_TOKEN`, the bearer token that makes the routine the gateway's requester.
- **Tests:** seven unit tests with fixed times (civil-date round trips against known dates; every-minute, hourly and daily schedules; weekdays, month ends, a leap day, both day fields, an impossible date; nine bad expressions; firing, skipping, disabling and resetting; loading and five bad documents) and two runner tests against a scripted agent served in-process on `rusty_serve` (a reply reported, a failing agent counted down to disabled).
- **Known limitations:** UTC only; no seconds, names, `L`, `W` or `#`; state is in memory; the reply is printed, not delivered to a channel (a routine that posts to Slack is the two crates composed, a later step).

## rusty_tick: calendar subscriptions, assignees, ambient sound, 3- and 10-day views
**2026-10-05** · [#525](https://github.com/Rusty-Mill/rusty_mill/pull/525)

- **Added:** `POST /api/v1/fetch-ics {"url"}` (`crates/apps/rusty_tick/src/fetch.rs`) and a Calendar subscriptions dialog: each feed (`https://` or `webcal://`) gets its own list, refreshes in place by the feed's UID, and refreshes on opening the calendar when older than six hours. The fetcher is HTTPS on port 443 only, refuses any non-public resolved address (checked before connecting, redirects re-checked, at most 3), 6 s per step, 4 MiB, body must be an iCalendar. `rusty_tick` now depends on `rusty_tls` (already in the workspace); `docs/WORKSPACE-MAP.md` regenerated.
- **Added:** `assignee` and `subscription` doc kinds; task assignees (free-text, a field, a row chip, an Assignee group in saved filters); 3-day and ten-day calendar views; ambient focus sound (white noise, rain, waves, synthesised with Web Audio).
- **Verified:** `cargo fmt --check`, `clippy -D warnings`, `cargo test -p rusty_tick`; web `tsc --noEmit`, `vitest` (640), `npm run build`; workspace layer, dependency and map checks.
- **Added:** `rusty_serve::Body::Deferred` / `Response::deferred`: a job run after the handler's lock is released, so the feed fetch holds up no other request; at most 4 fetches run at once (a fifth gets 503).
- Known limitations: a refresh overwrites the feed-owned fields of a task; no feeds with credentials; at most 500 events per feed; assignees are names, not accounts; not run against a live feed or in a browser.

---

## rusty_http and rusty_request: head cap, chunked body bound, no hidden pool replay
**2026-10-06** · [#526](https://github.com/Rusty-Mill/rusty_mill/pull/526) · consolidation review first batch B1, B2(a), B3

- **Fixed:** `rusty_http::head::parse_request_head`/`parse_response_head` enforce `max_head_len` on a head that completes, not only on one that has not; a terminated head over the cap is `HeadTooLarge`. Only the head's own bytes count, so body or upgrade bytes buffered after the blank line never trip it. All three transport adapters inherit the fix.
- **Fixed:** `read_body` on the `sync`, `async_tokio` and `tokio_native` adapters bounds a chunked body's decoded total by `DEFAULT_MAX_BODY_LEN` (1 MiB), as it already did for `Content-Length` and close-delimited framing, with a checked addition before each extend. `read_chunked_body` stays a line-bounded primitive with no aggregate cap and is documented as such.
- **Fixed:** `rusty_request` no longer replays a failed pooled-connection attempt on a fresh connection. That replay ignored method, retry policy and how far the request had got, so a `POST` the server accepted and then dropped could be submitted twice. The configured `RetryPolicy` is now the only replay authority; `send_streaming` keeps ignoring retry policy; a `Body::Stream` factory is opened zero times on a head-write failure and once otherwise.
- **Changed:** a `rusty_request` request with no retry policy that lands on a pooled connection the server has since closed returns `Error::Io` where it used to succeed on a second connection. Set `pool_idle_timeout` below the server's keep-alive timeout or configure a `RetryPolicy`.
- **Tests:** cap−1/cap/cap+1 for complete and fragmented heads with trailing body and upgrade bytes; chunked boundaries across chunk and read splits, extensions, trailers, empty body, premature EOF, and the unbounded primitive pinned; scripted-transport tests for head-write, partial head-write and post-body failures counting body-factory opens; loopback tests for a stale pooled connection without and with a policy, for streaming, and for a `POST` the peer accepts then drops reaching the server exactly once.
- **Known limitations:** no per-client body limit in `rusty_request` (a chunked response over 1 MiB is now an error); no idle-connection liveness probe, which is a separate improvement.

---

## rusty_channel: SMS over Twilio
**2026-10-05** · [#521](https://github.com/Rusty-Mill/rusty_mill/pull/521) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 5, third channel

- **Added:** `sms::Twilio`. Inbound, `X-Twilio-Signature` is checked in constant time against base64 of HMAC-SHA1 (hand-rolled on `rusty_sha1`, RFC 2202 vectors in the tests) over the webhook's public URL followed by every form field sorted by name; the channel is therefore constructed with the URL Twilio was given, and a webhook moved without telling it is refused rather than trusted. `From` and `To` make the conversation key (SMS has no threads), the trimmed `Body` is the text, and a message without text (media only) is ignored. Outbound, the reply is a `POST` to `Messages.json` with basic auth from the account SID and auth token, cut at Twilio's 1600-character limit with an ellipsis.
- **Changed:** `Channel` gains a defaulted `ack` hook: what the runner answers an accepted message with before the agent has run. Twilio wants TwiML, so its ack is an empty `<Response/>` as `text/xml`; the others keep the default `{}`. `bot::Bot` sends a non-JSON ack as a one-chunk `rusty_serve` stream, the server's way of carrying a chosen content type.
- **Added:** the `sms_bot` example on the shared runner.
- **Tests:** five, no network: the signature against an independently computed value and HMAC-SHA1 against RFC 2202; a signed text accepted, keyed, acknowledged and replied to; a missing header, a wrong signature, a changed body and a moved webhook refused; a media-only message ignored and long replies cut on a character boundary.
- **Known limitations:** no replay detection (Twilio signs no timestamp; a repeated `MessageSid` is not tracked); inbound media is ignored; no TwiML reply in the webhook response, the agent is too slow for that.

---

## rusty_tick: Won't Do, countdowns, pomo estimates, interruptions, .ics import
**2026-10-05** · [#520](https://github.com/Rusty-Mill/rusty_mill/pull/520)

- **Added:** `Status::WontDo` (`status: "wontdo"`, `?status=wontdo`), appended last so stored tasks still decode. Closing stamps `completedMs` and keeps the first stamp; the task menu has Won't Do and Mark as open; closed tasks list under Completed.
- **Added:** a Countdown page over a `countdown` doc kind; pomo estimates over an `estimate` doc kind (a stepper on the Pomodoro page and an estimated-against-actual list); an Interrupted button whose count is stored on the focus record.
- **Added:** `.ics` import from the Calendar view menu: events and todos become Inbox tasks (all-day end dates exclusive, `TZID` converted, cancelled/completed skipped, unsupported repeat rules imported once), capped at 500 per file.
- **Changed:** a shared `createDocStore` backs filters, countdowns and estimates.
- **Verified:** `cargo fmt --check`, `clippy -D warnings`, `cargo test -p rusty_tick`; web `tsc --noEmit`, `vitest` (616), `npm run build`.
- Known limitations: no calendar subscription by URL (needs a server-side fetcher); re-importing a file duplicates its tasks; an estimate for a purged task stays stored; a new web UI needs a new server (an old one rejects `wontdo` and the new doc kinds); doc saves are reported but not retried.

---

## rusty_channel: Microsoft Teams
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 5, second channel

- **Added:** `teams::Teams`, the Azure Bot Framework adapter. Inbound, the framework's bearer JWT is verified with `rusty_oauth`: `RS256` against the key the token's `kid` names in the framework's JWK Set, issuer `https://api.botframework.com`, audience equal to the app id, `exp` and `nbf` with five minutes of leeway, and a `serviceurl` claim that must match the activity's `serviceUrl`, so a forged activity pointing replies at another host is refused before its body is read. Only `message` activities become runs; `<at>` mentions are stripped; the Teams conversation id (which carries `;messageid=` for a channel thread) is the thread key. Outbound, a reply activity is posted to `{serviceUrl}v3/conversations/{id}/activities/{activityId}` with a token from the client-credentials grant.
- **Changed:** `Channel` gains two defaulted hooks. `credential(now)` returns `Ready` or `Fetch(Outbound)`, and `accept_credential(body, now)` takes the response: Teams' token is short-lived, so the runner fetches one before a reply and a minute before it expires. `reply_accepted(status, body)` lets a service that reports failure in a `200` body (Slack's `"ok": false`) say so. `Channel` is now `Send + Sync`.
- **Changed:** the runner moves from the Slack example into `bot::Bot` behind the `bot` feature, with `bot::get` and `bot::post` over `rusty_tls`; `slack_bot` and `teams_bot` are thin examples over it sharing an `examples/common` module. The Teams example fetches the framework's signing keys once at start.
- **Tests:** five for Teams, no network, against an RSA key and tokens produced with `openssl` outside the crate: a signed message accepted and keyed; no token, an altered payload, a wrong audience, an expired token, a not-yet-valid token, a token without `serviceurl`, an activity naming a foreign service, and an unknown key all refused; other activity types and bot messages ignored; the token fetch, its acceptance and expiry, and the reply URL, headers and body.
- **Known limitations:** keys are fetched once per process; a Bot Framework retry is not deduplicated (unlike Slack's, it carries no marker); outbound proactive messages are not supported, only replies.

---

## rusty_channel: chat channels for AG-UI agents, Slack first
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 5, first channel

- **Added:** `rusty_channel` at `crates/libs/protocol/rusty_channel`. `Channel` is sans-IO: `receive(headers, body, now)` authenticates one inbound request and returns `Challenge`, `Message(Inbound)` or `Ignored(why)`; `reply(to, text)` returns the `Outbound` `POST` (URL, headers, body) the runner sends. `Thread` keeps one conversation: `run(&inbound)` appends the person's message and builds a `RunAgentInput` carrying the whole thread and no tools; `absorb(events)` runs the events through the verifier and reducer and returns a `Reply` with the new assistant text and the error if the run ended in `RUN_ERROR`, a broken stream or an event out of order (the partial text is kept).
- **Added:** `slack::Slack`, the Events API adapter: `v0=` HMAC-SHA256 over `v0:<timestamp>:<body>` compared in constant time, a five-minute timestamp skew, `url_verification` answered, `app_mention` anywhere and `message` in a direct message accepted, a bot's own messages, subtypes, plain channel messages and Slack's retries ignored. A mention threads under itself so one Slack thread is one AG-UI thread; a direct message is one thread per person. Replies go through `chat.postMessage` with `&`, `<` and `>` escaped.
- **Added:** `examples/slack_bot.rs` behind the `bot` feature: `rusty_serve` in, `rusty_agui::HttpAgent` to the agent (or to `rusty_agent_gateway`'s `agui` route), `rusty_tls` out to Slack. The request is answered before the run (Slack wants a `200` within three seconds); runs are serialised per process and threads live in memory.
- **Changed:** `rusty_serve::Request` gains `headers`, every header of the request, and the crate re-exports `HeaderMap`.
- **Tests:** eleven, no network: thread building and absorbing (text, chunks, errors, a cut stream), an independent HMAC fixture, bad signature, stale clock, missing headers, a tampered body, the challenge, mention and thread keys, the direct-message key and reply body, every ignore case, text cleaning and escaping.
- **Known limitations:** Slack only; threads are not persisted; one run at a time per process; the bot example is not exercised in CI (it needs Slack credentials). Teams and SMS are the next two PRs of this step.

---

## rusty_agent_gateway: the `agui` route policy
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 4

- **Added:** `agentgateway-agui`, a crate beside `agentgateway-a2a`: `AguiGateway` compiles a route's `agui.rules` (CEL, the same language as `mcpAuthorization.rules`) and judges each run; `AuditedBody` wraps the proxied response and reports once, when the stream ends or the body is dropped; `audit::record_decision` and `audit::record_outcome` write the two records on the `agentgateway::audit` target.
- **Added:** `policies.agui` in `agentgateway-config` (`rules: [allow|deny|require]`) and its dispatch in the gateway: a `POST` is buffered (bounded like the A2A read), checked, recorded, then forwarded buffered; anything else on the route is proxied as is. Deny by default: no `allow` rule, no runs. Refusals are `403` with the reason; a `POST` that is not a `RunAgentInput` is `400`; `a2a` and `agui` on one route is a config error.
- **Rule context:** `agui.{threadId,runId,parentRunId,messages,lastUserMessage,tools,forwardedProps}`, `request.{method,path,headers}`, `jwt` (the claims `jwtAuth` verified).
- **Tests:** seven unit tests in the crate (default refusal, deny and require precedence, the context the rules see, a bad expression at build time, a non-POST and a bad body; the audited body passing bytes through and reporting finished, error, invalid, incomplete and disconnected runs) and five end-to-end tests against a mock agent through a running gateway (nothing reaches the agent on refusal; an allowed run streams through untouched; a GET passes through; the config pair is refused).
- **Known limitations:** audit records are tracing events, not a store; the gate reads the run input only, not the agent's response, so what the agent *did* is the outcome record's event count and ending, not its content.

---

## rusty_tick: the assistant, the React binding's first consumer
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 3, second PR

- **Added:** `rusty_tick::assistant`, an AG-UI agent served at `POST /api/agent` through `rusty_agui::AgentHandler`, behind the same bearer token as every other route (`Backend::authorize`; the streaming route lives in `server.rs` because it cannot return the buffered API response). It is deterministic and holds no store: it reads the thread, the context the UI exposes and the tools the UI offers, and answers as a pure function (`decide`). "add buy milk" becomes a `create_task` tool call the browser runs; a tool message answering it becomes "Added “buy milk”."; "what's due?" reads the open view; anything else gets help. An LLM-backed agent can replace it behind the same trait.
- **Added:** the web UI's Assistant panel (`src/features/assistant`, a rail button, `assistantOpen` in the UI store), on `@rusty-mill/agui-react`: `useReadable` for the open view and its tasks, `useAction` for `create_task` against the app's own store, `useAgent` for the thread. The agui packages are linked as `file:` dependencies; Vite dedupes React and inlines them in tests.
- **Tests:** three Rust tests on `decide` and one over a socket (a bad token gets 401, a good one streams a run with the tool call); two web tests through the real panel, store and hooks over a scripted agent (the task lands in the store and the confirmation shows; a 404 is shown and the panel stays usable).
- **CI:** the `rusty_tick web` and `rusty_tick e2e` jobs build the agui packages first; a change under `packages/` also selects the tick jobs.

---

## rusty_agui: the React binding
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 3, first PR

- **Added:** `@rusty-mill/agui-react` at `crates/libs/protocol/rusty_agui/packages/agui-react`, headless hooks over the core. `AgentProvider` holds one thread with one agent in a `useSyncExternalStore` store. `useAgent` gives messages, state, running and error plus `send`, `run`, `stop`, the thread's tool calls and `renderToolCall`. `useReadable` exposes application context for as long as the component lives. `useAction` registers a frontend tool: with a `handler` the agent's call is answered and a follow-up run starts; with only a `render` the call stays pending until the rendered UI calls `respond`, the human-in-the-loop pattern; `render` is the generative UI, given the parsed arguments, the call's status and its result. `useSharedState` sets state the next run sends and receives the agent's snapshots and deltas.
- **Tests:** four hook tests under jsdom against a scripted fake agent: a streamed reply and a `RUN_ERROR`; context and state sent and state received; a handled tool call answered and followed up with the tool message in the next run's thread; a render-only call that waits for the person and then continues.
- **Changed:** the `rusty_agui conformance` CI job typechecks, tests and builds the binding after the core.
- Not in this PR, by choice: the first consumer. Wiring an agent and a chat panel into `rusty_tick` is step 3's second PR, so it can be reviewed as an app change.

---

## rusty_agui: the headless TypeScript core
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 2

- **Added:** `@rusty-mill/agui-core` at `crates/libs/protocol/rusty_agui/packages/agui-core`, the headless TypeScript mirror of the crate with no runtime dependencies. Wire types and a validating `parseEvent` (unknown members kept, as the Rust codec ignores them); RFC 6901/6902/7386 pointers, atomic patch and merge patch; SSE `encode` and an incremental `Decoder`; a `Verifier` with the same ordering rules and chunk expansion as the Rust one; a `Reducer`; `streamAgent` (an async iterable of verified events) and `runAgent` (events, messages, state, result, outcome or error) over `fetch`. ESM, built with `tsc`.
- **Added:** `crates/libs/protocol/rusty_agui/fixtures/`: `events.json` (one sample per event type), `chunks.json` (chunk sequences and their canonical expansion) and `runs.json` (a whole run with its expected messages and state). The Rust codec, verifier and reducer tests and the TypeScript tests both read them, so the two implementations cannot drift apart without a test saying so.
- **Changed:** the conformance project also runs `@rusty-mill/agui-core` against the echo agent, beside `@ag-ui/client`; the `rusty_agui conformance` CI job runs the package's typecheck, tests and build first and consumes it as a `file:` dependency.
- **Verified:** `cargo test -p rusty_agui --all-features` (29, three of them the shared fixtures), `clippy -D warnings`, `fmt --check`; package `typecheck`, 16 unit tests and `build`; conformance 5 tests against the built example; workflow lint with pinned actionlint.
- Answers open question 1 of the follow-ons: the TypeScript core lives in-workspace under the crate, as the React apps do, so the `agui` planner flag covers it and the fixtures stay beside the Rust tests.

---

## rusty_agui: a client, and conformance against the reference TypeScript client
**2026-10-05** · [#515](https://github.com/Rusty-Mill/rusty_mill/pull/515) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md) · follow-ons step 1

- **Added:** `rusty_agui`'s `client` feature: `HttpAgent::new(url).run(&input)` posts a `RunAgentInput` and returns a `RunStream`, an iterator of verified events (through the same `Verifier` the server uses). Blocking, on `rusty_http`'s sync transport over `std::net`, plain `http://`; `Error::Status` for a refused run, `Error::Transport` for a closed socket, a non-SSE response or a stream that ends early.
- **Added:** the `echo_agent` example (`--features serve`): state snapshot and delta, a step, a tool call when the client offers a tool, an echoed assistant message; `fail` ends the run with `RUN_ERROR`.
- **Added:** `crates/libs/protocol/rusty_agui/conformance/`, a Node project where `@ag-ui/client` 1.0.2, the reference client CopilotKit's SDK and OpenBot embed, runs the example: a full run the reference client verifies and reduces (state, result, new messages), a frontend tool call with streamed arguments, and an agent failure delivered as `RUN_ERROR`. CI job `rusty_agui conformance (@ag-ui/client)`; planner flag `agui` (path and package).
- **Verified:** `cargo test -p rusty_agui --all-features` (27, plus the client doc test), `clippy -D warnings` with examples, `fmt --check`, the CI script tests (35), the three conformance tests locally against the built example, workspace map and layer checks.
- Two departures from the follow-ons document, recorded there: the client is on `rusty_http` directly rather than `rusty_request` (sync, mirroring `serve`; the gateway is async on its own stack), and the smoke test uses the reference `@ag-ui/client` rather than a browser-driven React app, since that client is what the React SDK drives agents with.
---

## rusty_baseline: honest measurement status
**2026-10-05** — [#428](https://github.com/Rusty-Mill/rusty_mill/issues/428)

- **Fixed:** The runner prints the full table and exits 1 if any measured stage fails. Unsupported rows, intentionally prebuilt binaries, and unavailable platform RSS counters are not failures. Other products and successful stages remain visible.
- **Fixed:** Exit-mode warm-ups and timed samples must match an exact expected code. Products default to 0; `@expect-exit=2` records the existing `ts-cli --help` contract, and `@expect-exit=linux:2` records Linux `ts-daemon --help` while leaving other hosts at 0 and preserving the Windows exclusion. A bad sample invalidates that product's runtime measurement; it cannot be hidden by a later successful sample. Idle mode retains its separate requirement to stay alive until intentional teardown.
- **Fixed:** The manual baseline workflow appends the partial report to the job summary and uploads it even when the measurement step fails, preserving the original failure status.
- Scope: no product implementation, dependency, support policy, or benchmark workload changed. Non-Linux `ts-daemon` remains a stub; any further host-support policy requires an owner decision.

---

## rusty_tick: Eisenhower matrix view and saved filters
**2026-10-05** · [#516](https://github.com/Rusty-Mill/rusty_mill/pull/516)

- **Added:** an Eisenhower matrix view mode (`viewMode: "matrix"`) beside list, Kanban and timeline. Important = any priority set; urgent = due by tomorrow or overdue. Dropping a card on a quadrant edits only the axes that differ (priority, due date). Kanban and matrix share one `TaskCard`.
- **Added:** saved filters: `filter` client documents holding a rule over lists, tags, priorities and due-date buckets (any-of within a field, all fields must match), a sidebar section with add/edit/delete, and a `/f/<id>/tasks` view with the usual grouping, sorting and view modes. The sidebar Filters placeholder is gone.
- **Changed:** `rusty_tick`'s README describes the web UI and the full route and field set; it had said "no web UI" and "not yet: recurrence, reminders".
- **Verified:** `cargo test -p rusty_tick`; web `tsc --noEmit`, `vitest` (594) and `npm run build`.
- Known limitations: a filter that fails to save is reported but not retried (habits are); filters are not cached offline; no Playwright e2e or real-binary integration test covers the new views; a filter cannot match "no tag" or use text or completion-state rules.

---

## rusty_agui: follow-ons document
**2026-10-05** · [#513](https://github.com/Rusty-Mill/rusty_mill/pull/513) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md)

- **Added (docs only):** [`docs/dev_phases/ADR_0007/FOLLOW-ONS.md`](docs/dev_phases/ADR_0007/FOLLOW-ONS.md) records where `rusty_agui` stands after #512 and plans the two targets set on 2026-10-05: frontend SDKs (React, Angular, Vue, iOS, Android) and an OpenBot-shaped agent platform (sandboxes, CEL gateway with pre-action audit, Slack/Teams/SMS channels, routines). It lists what the workspace already has and at which layer, the gaps, a ten-step build order starting with a Rust AG-UI client and a headless TypeScript core, four departures from the targets as named (one core then bindings; sandboxes on `rusty_rsi`'s executor rather than containers; CEL stays external; channels as adapters), and three open questions.

---

## rusty_agui and rusty_json_patch: a sovereign AG-UI stack
**2026-10-05** · [#512](https://github.com/Rusty-Mill/rusty_mill/pull/512) · [ADR-0007](docs/adr/0007-agui-and-json-patch.md)

- **Added:** `rusty_json_patch` (`crates/foundation`): RFC 6901 pointers, RFC 6902 patches with atomic apply and a `diff`, RFC 7386 merge patch, over `rusty_json::Value`; `no_std` + `alloc`; tests are the RFCs' own appendix vectors.
- **Added:** `rusty_agui` (`crates/libs/protocol`): the AG-UI protocol that CopilotKit's React SDK and OpenBot consume. All 31 event types, `RunAgentInput` and the message model; a hand-written JSON codec (unknown members ignored); SSE encode and incremental decode; a `Verifier` that enforces the ordering rules and expands chunk events as the TypeScript SDK does; a `Reducer` that folds events into messages and state; and, behind the `serve` feature, an `Agent` trait and `AgentHandler` for `rusty_serve` that frames a run, verifies every emitted event, and streams it.
- **Changed:** `rusty_serve` can stream: `Body::Stream` is written as chunked transfer encoding, one chunk per iterator item, after the handler lock is released. `rusty_tick` and `rusty_fair_play` switch to the new `Response::json` constructor.
- **Verified:** `cargo test` for `rusty_json_patch` (14, plus the `no_std` build), `rusty_agui --all-features` (24, including an end-to-end run over a socket), `rusty_serve` (7), `rusty_tick` and `rusty_fair_play`; `clippy -D warnings` on all five; `fmt --check`; `check_workspace_layers.py`, `check_workspace_deps.py` and the workspace-map regenerate.
- Known limitations, by choice (ADR-0007): no adapters yet from `rusty_adk`, `nexus-ai-runtime` or `rusty_key`'s own event types; no AG-UI client; no WebSocket transport; no CORS on the agent endpoint. `diff` is member-wise and index-wise, not an LCS diff.

---

## rusty_rsi: follow-ons document
**2026-10-05** · [#511](https://github.com/Rusty-Mill/rusty_mill/pull/511) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added (docs only):** [`docs/dev_phases/ADR_0005/FOLLOW-ONS.md`](docs/dev_phases/ADR_0005/FOLLOW-ONS.md) records where `rusty_rsi` stands after #509: the shipped PRs, the WSL2 steps for the first real-agent smoke run, the open follow-on options, the native Windows analysis (AppContainer plus Job Object, a probe first), and the decisions still needed.

---

## rusty_rsi: ADR-0005 configuration matches the code
**2026-10-05** · [#509](https://github.com/Rusty-Mill/rusty_mill/pull/509) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Fixed (docs only):** §8 said configuration came from `rsi.toml` with `RSI_*` overrides. There is no `rsi.toml`: `rsi` reads command-line flags and `RSI_*` environment variables, as §7 and the code already said (the workspace has no first-party TOML parser).
- **Fixed (docs only):** §6 and §8 said lineage records endpoint kinds. It records model ids only; a coding-agent CLI's id names the agent (`codex:<model>`, `claude:<model>`).

---

## rusty_orch: drop the planned Gemini adapter
**2026-10-05** · [#508](https://github.com/Rusty-Mill/rusty_mill/pull/508) · [ADR-0001](crates/apps/rusty_orch/docs/adr/0001-shared-substrate-over-agent-messaging.md)

- **Changed:** docs only. The Gemini CLI is discontinued, so the Gemini adapter is dropped, not deferred. `Agent::Gemini` stays a routing label so persisted records remain readable; no code change.
- Known limitation: a goal that routes to `gemini` still parses and fails at run time with "no adapter".

---

## rusty_rsi: record the outer agent's cost
**2026-10-05** · [#507](https://github.com/Rusty-Mill/rusty_mill/pull/507) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `outer_cost` on each proposal's lineage entry: the proposer's prompt and completion tokens, and the wall time the outer loop measured around the proposal. `rsi report` adds an **Outer cost** line beside the inner cost.
  - **Codex** proposals now run with `--json`; tokens are summed from the `turn.completed` events, as for `CodexModel`.
  - **Claude Code** tokens come from the JSON envelope's `usage`; prompt tokens include the cache writes and reads Claude Code counts apart.
  - A Codex or Claude run that reports no usage is now an error, never a free proposal.
- **Changed:** `CliProposer` owns a copy of the executor (it keeps 8 MiB of output for Codex's event stream) and has no lifetime parameter.
- **Compatibility:** entries written before this change keep their bytes and hashes, decode with no outer cost, and are counted apart in the report. A baseline with an outer cost is refused.
- **Tests:** codec round trip and legacy decoding, report totals, both fake agents metered, an unmetered Codex run refused, and the ten-step run recording a cost for every proposal. Mutation check: dropping the cost in the loop fails the run test.
- Known limitations: the outer cost is recorded, not budgeted; the budget bounds each inner run.

---

## rusty-mcp: public cache-hint helper; nexus-mcp sheds a private API and two dependencies
**2026-10-05** · [#505](https://github.com/Rusty-Mill/rusty_mill/pull/505) · [#479](https://github.com/Rusty-Mill/rusty_mill/issues/479)

- **Added:** `rusty_mcp::apply_cache_hints`, documented public API with a doc example. It sets `ttlMs` and `cacheScope` on a list result only for peers that negotiated spec 2026-07-28.
- **Changed:** `nexus-mcp`'s `list_tools`, `list_prompts` and `list_resources` call it instead of `rusty_mcp::__private::apply_cache_hints`, a `#[doc(hidden)]` module documented as not stable. `__private::apply_cache_hints` stays as a re-export, so `rusty-mcp`'s own macros are untouched.
- **Changed:** `nexus-mcp` drops its direct `http` and `reqwest` dependencies, which had no remaining uses after the Host client moved into `rusty-mcp` (#498), and trims its own `rmcp` features to `server` and `macros`. The client and Streamable HTTP client features move to `[dev-dependencies]` for the in-process adapter tests. Production builds still get them through `rusty-mcp`'s `client` feature, which already enables `client-side-sse` through the Streamable HTTP client feature. `Cargo.lock` loses the two edges.
- **Verified on the pinned 1.98.1 toolchain:** `clippy -D warnings` and `fmt --check` clean for `rusty-mcp`, `nexus-mcp` and `nexus-cli`; `rusty-mcp` 201 tests with the `client` feature and its doc tests with default features; `nexus-mcp` 108 tests; `nexus-bootstrap`'s `dep_invariants` 3 passed; `cargo check --all-targets` for every workspace crate that depends on `rusty-mcp`; `check_workspace_deps.py`, `check_workspace_layers.py` and the workspace-map verify. Not run locally: the full workspace sweep, because `libdbus-sys` needs headers this container lacks. CI on the PR is the authority for that.
- Out of scope, by choice: the Streamable HTTP option on `nexus mcp serve` (it is loopback-only with no auth and no Origin allow-list, and is awaiting an owner decision), and the proposal to let plugins publish their own tools through the dynamic registry.

---

## CI: retain scoped main work by application and check
**2026-10-05** — [#264](https://github.com/Rusty-Mill/rusty_mill/issues/264) — local review, no PR yet

- **Changed:** Main pushes start planning independently and retain pending jobs
  in component/check queues. Different applications can run concurrently; PR
  revisions still supersede the same PR. Cargo reverse-consumer selection and
  existing full-sweep coverage remain intact.
- **Added:** Strict queue-policy validation alongside pinned actionlint, which
  predates GitHub's `queue` key. See [ADR-0006](docs/adr/0006-ci-component-queues.md)
  for the 100-pending-job limit and exact-SHA validation contract.

---

## rusty_rsi: Codex as the inner model, Claude Code as the outer proposer
**2026-10-05** · [#504](https://github.com/Rusty-Mill/rusty_mill/pull/504) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `CodexModel` (`rsi-runtime::codex_model`), selected with `RSI_INNER_PROVIDER=codex`.
  - It is the inner agent's *model*, not the agent: a0 stays the thing the outer loop improves, and the broker still meters and records every call, so the budget hard stop and trajectory replay hold.
  - Each call runs one sandboxed `codex exec --json` in an empty directory and is charged from the `turn.completed` token usage. A call that reports no usage, or a failed turn, is an error.
- **Added:** Claude Code as the outer proposer, `RSI_OUTER_PROVIDER=claude` (binary from `RSI_OUTER_CLAUDE`, login in `CLAUDE_CONFIG_DIR`, model from `RSI_OUTER_MODEL`).
  - It runs `claude -p --restricted` with the file tools only (`Read,Edit,Write,Glob,Grep`), edits accepted and every other prompt denied, in the same sandbox and `.git`-free copy as Codex. No bypass flag is needed.
  - It uses the Claude subscription login; no Anthropic key passes through `rsi`.
- **Changed:** `rsi-runtime::codex` is now `rsi-runtime::agent_cli`: `CliProposer` and `CliConfig` (with a `CliAgent` of `Codex` or `Claude`) replace `CodexProposer` and `CodexConfig`. `ProcessExecutor::with_capture_bytes` sets how much output a run keeps.
- **Changed (breaking):** `RSI_OUTER_PROPOSER` is renamed `RSI_OUTER_PROVIDER`, matching `RSI_INNER_PROVIDER`. A leftover `RSI_OUTER_PROPOSER` is an error naming the new variable, never silently ignored.
- **Tests:**
  - **Fake agents.** Fake `claude` and `codex` scripts run through the real sandbox. Claude's edits come back, `.git` is untouched, and it cannot write outside or read private data; an `is_error` envelope and a login failure are errors. A Codex completion is charged exactly its reported tokens, and an unmetered or timed-out call is an error.
  - **Mutation checks.** Each check catches its mutation: an unmetered call charged as zero, and an ignored `is_error`.
  - **Real agents.** `#[ignore]`d tests run a real Claude Code proposal and a real Codex completion.
- Known limitations:
  - Codex has no per-call output cap, so one inner call may overrun the remaining token budget; the next call is refused.
  - Each inner call starts a Codex process.
  - The proposers' own token use is still not recorded.

---

## rusty_rsi: Codex CLI as the outer proposer
**2026-10-04** · [#485](https://github.com/Rusty-Mill/rusty_mill/pull/485) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `CodexProposer` (`rsi-runtime::codex`), selected with `RSI_OUTER_PROPOSER=codex`.
  - It runs `codex exec` inside `rsi`'s own sandbox. Codex's bubblewrap sandbox cannot start inside Landlock, so Codex runs in its documented external-sandbox mode (`--dangerously-bypass-approvals-and-sandbox`, plus `--ephemeral`, `--ignore-user-config` and `--ignore-rules`).
  - Codex edits a staging copy of the worktree that has no `.git`. It may write only that copy, `CODEX_HOME`, a private `TMPDIR` and `/dev/null`, and it uses the network.
  - It cannot reach the tasks, the repository or the run directory: the proposer refuses to start if its sandbox could.
  - Changes come back through the same no-follow, no-`.git` writer the model proposer uses, so the allowlist still decides.
  - Codex signs in with its own login; no key passes through `rsi`. Configuration: `RSI_OUTER_CODEX` (the native binary, default `codex` on `PATH`), `RSI_OUTER_MODEL` (optional) and `CODEX_HOME`.
- **Added:** `Sockets::Internet`, a sandbox socket rule for the Codex proposer only. It skips the internet-socket filter but keeps the process-group, `io_uring` and x32 locks.
- **Fixed:** rustils 0.27.2 (`platform-linux`). `Sandbox::confine_filesystem` accepted only directory roots: a file root failed with `EINVAL`. A file root now grants that file alone.
- **Tests:**
  - **Fake `codex`.** A fake `codex` script runs through the real sandbox. Its edits, additions and deletions come back. `.git` is untouched. It cannot write outside, read private labels or open devices other than `/dev/null` and `/dev/urandom`, while internet sockets work. Failures name their cause, such as `codex login`.
  - **Protected paths.** A protected path within reach stops the run before Codex starts.
  - **Mutation checks.** Each check catches its mutation: running without the internet rule, and dropping the protected-path check.
  - **Real Codex.** An `#[ignore]`d test runs a real, logged-in Codex.
- Known limitations:
  - Codex's token use is not recorded.
  - Network access is not restricted to the model's host.
  - Codex reads `/etc` whole.
  - No real Codex run was possible in CI or in the session that built this (no login).

---

## rusty_fair_play: choose the family deck, equal tiles, a collapsible side panel
**2026-10-04** · [ADR-0001](crates/apps/rusty_fair_play/docs/decisions/ADR-0001-front-end-shape.md) (decision 10)

- **Added:** a family chooses which cards are in its deck. `PATCH {"inPlay": false}` sets a card aside (taking it back from its owner in the same write); `true` adds it back. A set-aside card cannot be dealt, split or made a parent, and a split card cannot be set aside. The choice is a `set-aside.json` beside the stores, written by rename, so the stored `Card` keeps its layout and no data directory is migrated; it is part of the card's etag. The UI: an "In our deck" checkbox in the pane, a **Choose cards** mode with a checkbox tile per card and "All in / All out" per suit, a "Set aside · N" view to bring cards back, and the board, undealt list and balance counting only the cards in play.
- **Changed:** every card tile is the same size (it was as wide as its content) and styled after the printed deck — a blush-cream card, the name in spaced serif capitals, the suit written up the left edge in its colour (the look, not the artwork) — and the detail pane folds to a thin rail with a button, remembered across reloads; opening a card brings it back.
- Corrections: a set-aside card must be added back successfully before assigning an owner (a combined request is 422). Membership updates reach memory only after saving the sidecar; deletion/unsplit clean membership before removing cards so failed cleanup remains retryable. Confirmed and bulk deck changes retain selection-time ETags, and folding details preserves drafts and pending saves.
- Known limitations: the set-aside file is outside the engine's durability; operations across the card store and sidecar are not atomic and interruption can leave an unowned card in play. Only another app needing the choice would justify a stored `Card` field and a migration. Setting a split card aside needs an unsplit first.

---

## rusty_rsi P4: the outer loop, calibration and run reports
**2026-10-04** · [#483](https://github.com/Rusty-Mill/rusty_mill/pull/483) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `rsi run`, the outer loop. Each step:
  - checks the incumbent out in a sparse, detached git worktree that holds only the harness;
  - lets a proposer rewrite it;
  - commits the result under `refs/rsi/<run>/<step>`, never on a branch, and rejects it as a path violation if anything but a regular file under `harness/src/` changed;
  - builds it, grades it on fresh seeds, gates it (`screen`, then `confirm` on another fresh seed set) and appends the result to the run's lineage.
- **Added:** `rsi-runtime`:
  - `JsonlLineage`, an append-only, hash-chained `lineage.jsonl` with content-addressed `blobs/`. Every read re-verifies the chain and each entry's verdict.
  - A git adapter.
  - `ModelProposer`, whole-file rewrites from any chat model, and `ScriptedProposer`.
  - `outer::run` and `outer::calibrate`.
  - `report::summary` and `report::replay`.
- **Added:** `rsi calibrate`. It grades the base harness on N disjoint seed sets and writes the noise band and the margin `z·√2·σ̂`.
- **Added:** `rsi report [--replay]`. Replay re-grades every stored submission bit for bit and re-runs every inner run from its transcript.
- **Added:** `rsi-core` gains the `Proposer` and `LineageStore` ports. The proposer sees only `Precedent`s (verdicts, grades, public scores), so a per-task private score cannot reach it.
- **Changed:** model configuration is per role, from `RSI_INNER_*` and `RSI_OUTER_*` (`_MODEL`, `_BASE_URL`, `_API_KEY`).
- **Tests:**
  - **10-step run.** A 10-step run on the real suite in a throwaway repository, with scripted models, gets every verdict right: not better, within noise, path violation (manifest, symlink, private labels), buggy and accepted. The acceptance comes from a fresh, disjoint seed set.
  - **Replay and tampering.** Replay reproduces all 27 grades and 27 trajectories. An altered blob, a forged verdict and an overwritten run are all caught.
  - **Mutation checks.** The run test fails when the allowlist is removed, when re-evaluation reuses seeds, or when the proposer gets stale history.
- **Fixed (review):**
  - Proposer writes refuse a symlinked ancestor directory, not just a symlinked leaf, so a write cannot escape the worktree.
  - Refs are create-only, and a run claims `refs/rsi/<run>/base` first: a second run directory with the same name is refused instead of overwriting the first run's refs.
  - A missing or baseline-less lineage is an error, not a successful replay. A partial run is reported as `INCOMPLETE`, and `--replay` on it fails after checking what was recorded.
- Known limitations:
  - The outer model must be a local OpenAI-compatible endpoint (there is no TLS yet). A Codex CLI proposer is deferred.
  - The parent is always the incumbent.
  - The outer model's token cost is not yet recorded in lineage.

---

## rusty_fair_play: a web front end for the Fair Play domain
**2026-10-04** · [ADR-0001](crates/apps/rusty_fair_play/docs/decisions/ADR-0001-front-end-shape.md) · follows [ADR-0137](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0137-fair-play-domain.md)

- **Added:** `rusty_fair_play` (`crates/apps/rusty_fair_play`), a JSON HTTP API over the embedded `fair_play` stacks on `rusty_http`/`rusty_json`/`rusty_url` (rusty_tick's sans-IO router and thread-per-connection adapter): one boot read with every card's derived state, people, card patch/create/split/reset/baseline/position, and an idempotent `/seed`. The deck ships in the binary and loads on first start. An optional bearer token; without one the server is loopback-only.
- **Added:** the web UI in its `web/`: React/TypeScript/Vite/Tailwind/Zustand with hash routes — a deck board in six suit shelves with owner chips and `edited`/`custom`/split badges, filters and search; a card pane with dealing, CPE editing, minimum standards, notes, a baseline diff and reset, ordered children and a split dialog; a players page; a balance page with all-cards and leaf-only bars and the "still undealt" leaves. An in-browser `MemoryAdapter` runs the same rules for unit tests and a demo mode; the contract runs against the real binary; Playwright drives the built UI against it. Two CI jobs mirror rusty_tick's.
- **Changed:** the Fair Play domain is its own libs crate, `rusty_fair_play_domain` (`crates/libs/storage/`), re-exported by `rusty_multimodal_db` as `generic::fair_play` the way the engine is (ADR-0124), so the app depends on no other app crate (ADR-0003's layer rule). The seed loader moved from `examples/support/` into it as `seed` (pure text parsing, file wrappers beside it, `DECK_CSV` embedded); the seed CLI, the benchmark example, the crash writer and the domain's tests moved with it; the `fair_play_seed` example's `--cards` is now optional.
- **Added:** the gaps the first cut named, closed. The domain crate gains guarded deletes — `delete_card` (leaves only), `unsplit_card` (ADR-0137's merge/unsplit hook: the subtree, deepest first, parent kept), `delete_person` (holding nothing) — and `reorder_children` (an exact list of the children, validated before anything is written). The API exposes them (`DELETE /cards/{id}`, `POST …/unsplit`, `PUT …/children/order`, `DELETE /people/{id}`; 409 when a guard refuses). Every card carries an `etag`, and a `treeEtag` over its whole subtree; a write may send the right one as `If-Match` (the subtree tag for unsplit and reorder) and a mismatch is 412 with the current card. The web UI uses all of it: delete and unsplit with confirmation, re-parent and suit selects, one-request child reorder, the full 100-card deck in demo mode, and `If-Match` on every card write using the version the edit was based on, with a kept draft and an Overwrite / Discard notice on a conflict.
- **Added:** `rusty_serve` (`crates/libs/net/rusty_serve`): the blocking `rusty_http` HTTP/1.1 server and path-safe static file loader that rusty_tick and rusty_fair_play both carried, now one crate with a sans-IO `Handler` trait; each app's `server.rs` is a few lines binding its router to it.
- **Fixed (review):** the `seed` subcommand now takes the directory lock, so it is refused while a server holds the directory (a separate-process test checks the refusal and that no file changed). "The deck is loaded" is a `deck.loaded` marker written after a full load, not the presence of card 1, so a deleted deck card stays deleted across restarts and an interrupted first load finishes on the next start. In the browser, a save no longer discards what was typed while it was in flight, and a slow or out-of-order snapshot can no longer roll back an acknowledged write or drop a new card. `rusty_multimodal_db`'s Fair Play server adapter maps the new `CardError` variants.
- Known limitations: a child reorder is one validated request, but the positions are then written one by one, so it is not crash-atomic (equal positions fall back to id order). `If-Match` is optional on the wire; a client that omits it still wins last-writer. Conflicts are per card, not per field. `POST /seed` also restores a deleted deck card.

---

## rusty_rsi P3: the inner agent a0, its broker and model clients
**2026-10-04** · [#478](https://github.com/Rusty-Mill/rusty_mill/pull/478) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `crates/apps/rusty_rsi/harness` (`rsi-harness`), a0, the std-only inner agent. It ports AIDE0: five drafts, then debug a random buggy leaf (p = 0.5, debug depth at most 3) or improve the best node, with the full history in every prompt; it submits each new best. The runtime compiles `src/lib.rs` as a binary with plain `rustc` in the sandbox, so a candidate has no manifest, dependencies or build scripts. A compile error is a build failure, not a crash.
- **Added:** `rsi-runtime`:
  - The broker: a length-prefixed binary protocol (`llm`, `eval`, `submit`) over a socket pair passed as the agent's stdin. `LiveService` meters model tokens and wall-clock time with `CostMeter`; once the budget is spent only `submit` works, and the agent is killed at the wall-clock budget plus a grace period.
  - Every exchange is recorded. `HarnessProcess::replay` re-runs an agent against its transcript, without the model, and fails on the first divergent request.
  - `OpenAiModel`, an OpenAI-compatible client over `rusty_http` (plain HTTP; local Ollama by default), and `ScriptedModel` for CI. A response without token usage is refused.
  - The agent's sandbox refuses every new socket, so the broker socket is its only channel.
- **Added:** `rsi inner`, one inner run with a live model, configured only from the environment (`RSI_INNER_MODEL`, `RSI_INNER_BASE_URL`, `RSI_INNER_API_KEY`).
- **Added:** `rsi-core` gains the `Harness` and `ChatModel` ports and `PublicTask::description`; each toy task gains `public/task.md`.
- **Tests:** end-to-end tests for invariant 2 (the token budget stops a0 after exactly the affordable calls; an agent that ignores the wall clock is killed and keeps its submission), invariant 4 (a run replays to the same submission without the model; a changed task diverges) and invariant 1(b) (the agent cannot read private or public labels, the task or the repository, or open TCP or Unix sockets). A mutation check confirmed the isolation and budget tests fail when the socket rule or the budget check is removed.
- **Security (review):**
  - **Socket rules.** The agent can create no socket of any kind; `socketpair` is now refused too. The build refuses `socket` but keeps anonymous socketpairs, which rustc needs to start its linker. Every sandbox, solutions included, refuses `io_uring`, which would bypass seccomp.
  - **Transcript cap.** The broker's transcript is capped at 64 MiB, charging each exchange its frames plus a fixed overhead. The cut is deterministic, and the last accepted submission is kept.
  - **Deadline.** Model calls and evaluations get the budget's remaining time as a hard limit. A call cut off at the deadline counts as an exhausted budget, so the run keeps its earlier submission.
  - **Response size.** Every HTTP response framing, chunked included, is capped at 16 MiB while it is read.
  - **API key.** With a key configured, nothing the endpoint sent reaches a diagnostic: neither an error body nor a head, framing or body parser error that quotes it.
  - **DNS.** The model endpoint is resolved once, when the client is built, under a timeout. Only one lookup may run at a time, so a stalled resolver cannot pile up threads. Connecting uses the call's remaining time.
  - **Tests and mutation checks.** Each fix has regression tests, and each test was confirmed red with its defence removed.
- Known limitations:
  - `https://` model endpoints are refused until TLS is wired in.
  - The last model call can overshoot the token budget by its prompt tokens (admit-then-record).
  - Solutions, unlike the agent, may still create Unix sockets.

---

## rusty_multimodal_db: the Fair Play domain (ADR-0137)
**2026-10-04** · [ADR-0137](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0137-fair-play-domain.md) · no wire change

- **Added:** `generic::fair_play` — Eve Rodsky's household-task cards as three tables on the one-index/one-scan stack: `Person`, `CardDefault` (the shipped text, read-only by convention) and `Card`, a self-referential tree with an explicit owner per card, CPE as three fields, and a state (`Original`/`Edited`/`Custom`) derived against the baseline rather than stored. Queries over the generic traits: held by, unassigned and unassigned leaves, by suit, by number, balance (all or leaf-only), reassign, ordered children, chain to root, nested tree, leaves under, owner coverage, split, custom card, state, diff, reset, counts.
- **Added:** `split_card`, ordered so every crash prefix is a valid store (children first, parent last), proven by `tests/fair_play_crash.rs` with a real `SIGKILL` after each step; the seed loader `examples/fair_play_seed.rs` for the supplied 100-card deck (hand-rolled CSV, refusals by file and line, idempotent by deterministic id, never overwriting a family's edits); `examples/fair_play_bench.rs`.
- **Added:** `server::fair_play` — `card`, `person` and `card_default` adapters through `serve_tables`, `fair_play_server`, the socket suite and a Python driver against the three-table server.
- **Changed:** `rusty_multimodal_db_engine`: `Reversed::inner`, so a stack with two `Reversed` layers reaches the inner one's `Children`.
- **Measured:** a depth-5 parent-chain walk costs 1.3 µs against 160 ns for one read, so no denormalized `root_card_id`; state queries scan at about 1 µs a card (100 cards 0.1 ms, 5 000 cards 18 ms), so no second index.
- Known limitations: the stack cannot enforce `number` uniqueness, acyclicity or the origin/number/baseline invariant (the domain functions do; the raw traits bypass them, tested); the wire carries one relation per table, so `owner_id` is a filterable field there, not an index; `card_default` is read-only on the wire and by convention in-process; no delete in the domain (merge/unsplit and deal history are named hooks).

---

## rusty_rsi P2: sandboxed execution, toy tasks and private grading
**2026-10-04** · [#476](https://github.com/Rusty-Mill/rusty_mill/pull/476) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `rsi-runtime`:
  - `ProcessExecutor` runs untrusted programs through the `rsi __sandbox` helper. The helper applies rlimits, then Landlock with a read allowlist, then a seccomp block on internet sockets, then `exec`s the program. Setup failures come back through a close-on-exec status file, so a run fails closed.
  - Wall-clock kills take down the whole process group.
  - `TaskDir` (`task.json`), `LocalTask` (public scoring) and `SandboxedGrader` (private grading through the separate `rsi __grade` process).
- **Added:** three toy tasks, one per family. Each has public and private splits and a naive baseline, generated deterministically by `tasks/generate.py`:
  - `ml-regression` (R²)
  - `tsp-heuristic` (tour ratio)
  - `scaffold-oracle` (accuracy around a noisy oracle)
- **Tests:** end-to-end tests for invariant 1 (a solution cannot read private labels during public or private runs; the runner refuses a sandbox that could reach the task) and invariant 5:
  - internet sockets are denied, and so are writes outside the work directory;
  - memory, CPU and wall-clock limits are enforced;
  - sandbox setup failures fail closed.

  A mutation check confirmed these tests fail when the confinement or the rlimits are removed.
- **Security (review):**
  - **Output ingestion.** A solution's output is read once, without following links or blocking. Only a regular single-link file within the size limit is accepted, and that snapshot goes to the private grader on stdin.
  - **Containment.** A seccomp filter forbids `setsid` and `setpgid`, so the process group is the whole job. The executor kills it and verifies through `/proc` that no live member survives.
  - **Regression tests.** Symlink and FIFO outputs, input swapping, and detached grandchildren at normal exit and at timeout. Each was confirmed red before the fix, and red again when its defence is removed.
- **Fixed:** `rsi-runtime` now really has no external dependencies. The workspace's `rusty_json` entry was silently re-enabling the serde default feature.
- Known limitations:
  - Linux only; elsewhere every run fails closed.
  - `RLIMIT_NPROC` does not bind root.
  - The harness-family oracle is a readable simulator until the P3 model broker exists.

---

## rusty_tick: calendar test no longer fails on Sundays
**2026-10-04** · [#474](https://github.com/Rusty-Mill/rusty_mill/pull/474)

- **Fixed:** `CalendarPage.test.tsx`'s repeating-task test anchored a daily repeat at today in a Monday-start week view; on a Sunday, the week's last day, the task rendered once and `expected 1 to be greater than 1` failed. The repeat now starts on the visible week's first day. Verified with the clock pinned to each weekday: the old test failed only on Sunday, the new one passes on all seven. Test-only; no app code changed. The same test change first reached `main` ported into #472.

---

## rusty_rsi P1: a self-improvement harness's pure core
**2026-10-04** · [#472](https://github.com/Rusty-Mill/rusty_mill/pull/472) · [ADR-0005](docs/adr/0005-rsi-harness.md)

- **Added:** `crates/apps/rusty_rsi/crates/rsi-core`, the I/O-free domain of an AIDE²-style self-improvement loop: `Score`/`Grade` confined to `[0, 1]`, a token + wall-clock (+ GPU) `CostMeter` with a hard stop, `NoiseBand` and a `z·√2·σ̂` accept `Margin`, a two-stage fresh-seed accept gate (`screen`/`confirm`), `argmax`/UCB1/softmax helpers, a SplitMix64 PRNG with counter-based seed derivation, and lineage entry types with a SHA-256 hash chain.
- **Changed (review):** lineage entries are now validated: `LineageEntry::new` replays the gate on the recorded evaluations and refuses a decision they do not produce; a repeated `(task, seed)` result is rejected rather than double-counted; softmax stays correct at finite extremes (`[f64::MAX, -f64::MAX]` at `T = f64::MAX` gives ≈ `[0.881, 0.119]`, not `[1, 0]`) and for subnormal logits (`[5e-324, 0]` at `T = 5e-324` gives ≈ `[0.731, 0.269]`, not `[0.5, 0.5]`).
- **Docs:** ADR-0005 accepted.
- Known limitation: nothing runs yet. Tasks, the sandbox, the inner harness and the outer loop arrive in P2 to P4.

---

## Foundation spinlocks share one implementation
**2026-10-02** · [#411](https://github.com/Rusty-Mill/rusty_mill/issues/411)

- **Changed:** `rusty_std::sync::Mutex` is the sole synchronous atomic spinlock
  mechanism. The existing `rusty_sync::SpinLock` and guard safely delegate to
  it through private fields.
- **Compatibility:** both public paths remain available as nominally distinct
  lock and guard types, so downstream local-trait implementations remain
  separate. Acquisition, release, auto-trait bounds, and non-poisoning behavior
  are unchanged; no fairness, async-awareness, or interrupt-safety guarantee
  was added.

---

## rusty_h2: remove dormant connection sources
**2026-10-02** · [#418](https://github.com/Rusty-Mill/rusty_mill/issues/418)

- **Removed:** three uncompiled, undeclared `connect` source files that duplicated live settings and PING responsibilities and contained an incorrect server-preface model. Compiled behavior and public API are unchanged.
- **Docs:** architecture now describes the existing connection driver and thin client/server wrappers while retaining the crate's no-I/O boundary.
- Known limitations: byte-level connection-preface validation and scheduled keepalive are not implemented by this cleanup.

---

## rusty_baseline: explicit platform eligibility
**2026-10-02** · [Issue #428](https://github.com/Rusty-Mill/rusty_mill/issues/428)

- **Fixed:** `products.txt` can mark a product with `@platform=linux`, `@platform=windows`, or `@platform=macos`, and can narrowly exclude one OS with `@unsupported=<os>`. A selected product that is unsupported on the current OS is kept in its original report position as an explicit skipped row with unmeasured cells, and it is filtered before any Cargo query, build, binary lookup, or process launch. Existing entries without a declaration remain unrestricted. The already Linux-only `rusty_fedora_agent` retains `@platform=linux`; `ts-daemon` now uses `@unsupported=windows` temporarily, until it has a real Windows implementation rather than its current stub, without making a policy choice for other hosts.
- Known limitations: this does not change aggregate exit status, expected-exit handling, or Ubuntu baseline prerequisites.

---

## rusty_multimodal_db: chunked snapshots, a standby for a table over 8 MiB (ADR-0136)
**2026-10-01** · [ADR-0136](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0136-chunked-snapshots.md) · wire protocol 35 → 36

- **Added:** `BeginSnapshot`, `FetchChunk`, `EndSnapshot`, `SnapshotManifest`, `Chunk` and `ErrorCode::NoSnapshot`. The server copies a table's files to `SERVER_SNAPSHOT_DIR` under the write lock, then serves them in chunks of at most 4 MiB with a SHA-256 per file while writers run. `SchemaDrivenClient::fetch_snapshot_chunked`, `replica_refresh` (falls back to it when `FetchSnapshot` answers `TooLarge`), the Python client, `examples/snapshot_stall_bench.rs`.
- **Changed:** `SERVER-001` 0.109.0, `SERVER-002` 0.25.0, protocol 36. `FetchSnapshot` is unchanged. Opt-in: with no `SERVER_SNAPSHOT_DIR`, `BeginSnapshot` answers `Unsupported`.
- **Measured:** writers wait for the staging copy, 4.4–4.7 s at 1 GiB on the test disk (0.09 s at 128 MiB; 0.2–3.0 s at 512 MiB, depending on the page cache). The default ceiling is `SERVER_SNAPSHOT_MAX_MB=1024` for that reason.
- **Fixed (review):** a manifest file name such as `C:escape` passed the client's check and, on Windows, could be written outside the download directory. Both clients now share one portable rule (`is_plain_file_name`, decided on the text, so Linux tests cover Windows-style names) and check every name before creating any file; `replica_refresh`'s legacy check uses it too.
- **Added:** `rusty_libc::fs::statfs` and `Statfs::available_bytes`; a snapshot is refused (`Storage`) before copying when the staging disk has less free than the table plus 64 MiB.
- Known limitations: nothing above 1 GiB was measured; the free-space check runs on Linux only and a disk that fills during the copy still fails the snapshot (`Storage`); the process-kill crash trials in the ADR were not run (the startup sweep and the cleanup paths are unit-tested); a dropped connection loses its snapshot and the standby starts over.

---

## rusty_multimodal_db: chunked snapshots proposed (ADR-0136)
**2026-09-30** · [ADR-0136](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0136-chunked-snapshots.md)

- **Docs:** a proposal to lift the 8 MiB snapshot cap: stage a consistent copy under the write lock, then stream it in chunks (protocol 36), with per-file SHA-256. No code.
- **Measured:** a locked local copy stalls writers about 6 s per GiB on the test host, which is why the lock covers the copy and never the transfer.

---

## rusty_multimodal_db: a grouped change-log sync, spiked on Memory
**2026-09-30** · [ADR-0134](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0134-grouped-change-log-sync.md)

- **Added:** `ChangeLog::append_deferred` and `sync_through` (append without `fsync`, then one leader syncs for the group), `MemoryConnectionStore::with_change_log`, and `sink` rows in `change_log_bench`. Proposal only: the server does not use it, and `ChangeLogged` is unchanged.
- **Measured:** journaled updates at 16 writers, 10.1k ops/s with the sink against 2.5k with the decorator (14.9k with no log). Insert-dominated tables lose nothing to the log.
- Known limitation: only `Memory`'s atomic `write_batch` is wired; `Entity`, `Relation` and the single-shot write paths are not, so a table served this way must not use the sink yet.

---

## rusty_multimodal_db: what the change log costs under load
**2026-09-30** · [ADR-0131](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0131-continuous-replication.md)

- **Added:** `examples/change_log_bench.rs`, a multi-threaded write benchmark with and without the change log, journaled or not (`--example change_log_bench`, release build).
- **Measured:** the log adds one `fsync` per write (about −40%) and, on an update-heavy journaled table, removes group commit: 17.2k against 3.1k ops/s at 16 writers on one 4-core ext4 host. Recorded in ADR-0131 with a proposed cheaper design.
- Known limitation: one host, one disk; the ratios are the result, not the absolute numbers. Nothing in the log was changed.

---

## Design review Tranche 1: soundness and safe-API contracts
**2026-09-30** · [#409](https://github.com/Rusty-Mill/rusty_mill/pull/409)

- **Fixed:** `rusty_std` `MutexGuard<Cell<_>>` was `Sync`; it now requires `T: Sync`, and compile-fail doctests cover both guards.
- **Fixed:** `rusty_sync` channels could report `Disconnected` with the final value still queued.
- **Fixed:** `rusty_rand` on Windows reported success after a truncated fill of a buffer over 4 GiB.
- **Fixed:** `kill_single` (Linux, async Linux, Windows) no longer signals a reaped child; it returns `Ok`, like `std`.
- **Changed (breaking):** `OwnedWinHandle::from_raw` and `rusty_libc::process::process_vm_writev` are now `unsafe fn`.
- **Docs:** remediation plan at `docs/Monorepo_Reviews/DESIGN-REVIEW-2026-09-30-PLAN.md`.

---

## rusty_multimodal_db: documentation brought up to date
**2026-09-30**

- **Docs:** `README.md`, `AGENTS.md`, `WORKFLOW.md`, `clients/python/README.md`, `docs/architecture/SYSTEM-ARCHITECTURE.md`, `docs/PROJECT-STATUS.md`, the specs and the traceability matrix now describe the crate as it is: a monorepo member whose storage core lives in `rusty_multimodal_db_engine`, wire protocol 35, seven adapters, the commands CI actually runs. About 170 file paths in the specs, `TRACEABILITY.md` and the registry that still named the pre-extraction `src/generic/*` locations now point at the engine crate; three ADR titles that still said "(Proposal)" were corrected.
- Known limitation: about 270 stale paths remain in `ROADMAP.md`, `PROJECT-STATUS.md`'s dated entries, `RESULTS.md`, the design docs and older ADRs. Those are records of what was true when written and were left as written. The audit is `crates/apps/rusty_multimodal_db/docs/reports/DOCS-AUDIT-2026-09-30.md`.

---

## rusty_tick: type-ahead search
**2026-09-30**

- **Changed:** `GET /api/v1/search?q=` treats each term as a prefix (`grocer` finds `groceries`), using the engine's `Query::any_of_prefix`. Terms are still alternatives; a prefix, not a substring.
- **Docs:** README no longer lists moving a task between lists or accounts as missing (both exist); `docs/FUTURE-GROWTH.md` records what the `rusty_tick` spike (#382) changed in the engine.
- Known limitation: finding prefix terms scans the index vocabulary, fine for a task corpus and a cost to weigh for a very large one.

---

## rusty_tick: `user adopt`, and `user` commands no longer race
**2026-09-30** · [ADR-0002](crates/apps/rusty_tick/docs/decisions/ADR-0002-per-user-tokens.md)

- **Added:** `rusty_tick user adopt KEY [--data-dir DIR]` moves a single-user store into `users/KEY/`, creates the user and prints a token, so existing data keeps working under a per-user token. It needs the server stopped and refuses if there is nothing to adopt or the user's directory exists.
- **Fixed:** two `user` commands at the same moment could lose one write. Commands that edit `users.json` now hold `users.lock`; a second one refuses instead of merging.

---

## rusty_tick: the `user` commands (ADR-0002 step 4)
**2026-09-29** · [ADR-0002](crates/apps/rusty_tick/docs/decisions/ADR-0002-per-user-tokens.md)

- **Added:** `rusty_tick user add KEY [LABEL] | list | revoke KEY TOKEN_ID | disable KEY | enable KEY [--data-dir DIR]`, which edit `users.json` and exit. A running multi-user server sees the change within a second.
  - `add` creates `users.json` in a fresh directory, or gives an existing user another token. It prints the token alone on stdout, once, so `TOKEN=$(rusty_tick user add alice phone)` works.
  - Failures go to stderr with a non-zero exit and print nothing on stdout.
  - `add` refuses a directory that holds a single-user store (`tasks.mmap`), which a new `users.json` would stop serving. `user adopt` (below) moves it.
- **Added:** `rusty_tick::admin::run`, the command layer, over `users::Registry`. No new dependencies.
- Known limitations: a running server is not told, it re-reads the file.

---

## rusty_tick: several users on one server (ADR-0002 step 3)
**2026-09-29** · [ADR-0002](crates/apps/rusty_tick/docs/decisions/ADR-0002-per-user-tokens.md)

- **Added:** multi-user mode. If `<data-dir>/users.json` exists, `rusty_tick` serves several users.
  - A token is `<user key>.<secret>`; each user's data is `<data-dir>/users/<key>/`, at most 32 open at once.
  - `users.json` is re-read as it changes, so a revoked token stops working without a restart; a file that stops parsing keeps the last good one and is reported once.
  - Every refusal is the same bare `401`: a missing header, another scheme, an unknown or disabled user, a revoked token and a wrong secret. The server logs the user key a refused token claimed, never the secret.
  - `503` if another process holds a user's directory. `/health` needs no token and opens no store.
  - `RUSTY_TICK_TOKEN` must not be set in this mode.
- **Added:** `rusty_tick::backend::Backend` (authentication, then the caller's data; no sockets) and `rusty_tick::auth::Authenticator`.
- **Changed:** `Server::bind` takes a `Backend` instead of an `Api` and a `Service`. `Api` is now three steps (`public`, `authenticate`, `serve`) that `Backend` runs, and `Api::handle` is gone. `UserKey` moved from `pool` to `users`, with its own error instead of `PoolError::InvalidUser`, so `users` no longer reaches into `pool`.
- **Fixed:** single-user startup took no lock on the data directory, so two servers on one directory overwrote each other. Both modes now hold a `DirLock` and a second server refuses to start.
- Single-user mode is otherwise unchanged: `RUSTY_TICK_TOKEN`, the data directory itself.
- Known limitations: nothing creates `users.json` yet (the `rusty_tick user ...` commands are step 4); no rate limiting (left to the TLS front end); opening an evicted user's store happens under the server's one lock.

---

## rusty_tick: user registry and token check (ADR-0002 step 2)
**2026-09-29** · [ADR-0002](crates/apps/rusty_tick/docs/decisions/ADR-0002-per-user-tokens.md)

- **Added:** `rusty_tick::users`, not yet wired to the API.
  - `Token::parse` reads `<user key>.<secret>`; a secret is 32 `rusty_rand` bytes as unpadded URL-safe base64.
  - `Registry` holds users and the SHA-256 digests of their tokens, never the secrets: `add_user`, `add_token` (returns the full token once), `revoke`, `set_disabled`, and `authenticate`.
  - `authenticate` returns `None`, and does the same work, for a malformed token, an unknown or disabled user, a revoked token and a wrong secret.
  - `users.json` is versioned and refused if it has an unknown field, a bad key, a duplicate user or token id, or a digest that is not 64 hex characters. `save` writes a temporary file, syncs and renames it, `0600` on Unix.
  - `RegistryFile` re-reads the file when its modification time or length changes (at most once a second by default), and keeps the last good registry if a later edit does not parse, reporting why through `last_error`.
- **Added dependencies:** `rusty_rand`, `rusty_base64` and `rusty_rsa` (for its SHA-256), all first-party. No external dependency and no new package in the lockfile.
- Known limitations: no rate limiting (ADR-0002 leaves it to the TLS front end); the CLI that edits the file is step 4.

---

## rusty_tick: `ServicePool` (ADR-0002 step 1)
**2026-09-29** · [ADR-0002](crates/apps/rusty_tick/docs/decisions/ADR-0002-per-user-tokens.md)

- **Changed:** `pool::StorePool` is now `pool::ServicePool`, and pools a user's whole `Service` (tasks and lists) instead of only a `TaskStore`. `StorePool` could not serve the API, since a user's data is both.
  - `ServicePool::new(root, capacity, clock)` takes a factory for the clock each opened service reads.
  - `DEFAULT_MAX_OPEN_USERS` is 32, a constant (ADR-0002).
- No behaviour change: nothing calls the pool yet. The binary and API are untouched.
- Known limitation: single-user startup still takes no `DirLock` (ADR-0002 proposes fixing it with the authenticator step).

---

## rusty_multimodal_db_engine and rusty_tick: growth follow-ups (issue #382)
**2026-09-29** · [#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)

- **Added:** `Query::except_columns`, FTS5's `- {c1 c2} : (query)` (every column but those). Held to FTS5 by `tests/fulltext_vs_fts5.rs`; nests with `in_columns` by intersection.
- **Added:** `rusty_tick::pool::StorePool`, a bounded pool of per-user stores.
  - Keeps at most `capacity` stores open and closes the least recently used first, before opening the next, so the bound holds while opening.
  - Locks each user's directory with `DirLock`, since the store does not lock by itself.
  - User keys are validated (1 to 64 of `A-Z a-z 0-9 _ -`), so a key cannot leave the pool's root.
- **Known limitation:** nothing calls the pool yet. The HTTP API has one bearer token and no user identity, so wiring it in needs per-user tokens first.

---

## rusty_multimodal_db_engine: column filters and store lifecycle (issue #382)
**2026-09-29** · [#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)

- **Added:** `Query::in_columns` and `Query::in_column`, FTS5's `{c1 c2} : (query)`.
  - The filter applies to phrases, so `all_of` under it needs each phrase in a chosen column; a column the index lacks matches nothing; nested filters intersect.
  - Held to FTS5 by `tests/fulltext_vs_fts5.rs` (ranking and snippets), including prefix, `AND` and `NOT` inside a filter.
- **Added:** `tests/store_lifecycle.rs` for one store directory per user.
  - Closing loses nothing and a reopen sees every write.
  - `DirLock` refuses a second handle in the same process, which is what makes closing an idle store safe; the store itself does not lock.
  - An ignored probe measures open, close and idle memory: about 1.5 µs and 260 bytes per record (2.3 ms and 257 KiB at 1,000 records). Numbers are in `rusty_tick`'s `SPIKE-FINDINGS.md`.
  - No engine API added: the pool of open stores stays app-side.
- Known limits: the measurements are one machine, one run; column filters cover inclusion only, not FTS5's `- col :` exclusion.

---

## rusty_multimodal_db_engine: task-manager gaps (issue #382)
**2026-09-29** · [#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)

- **Added:** additive `fulltext::Query` forms, leaving `Query::any_of` and its results unchanged.
  - `Query::any_of_prefix` matches the last token of each phrase as a prefix (`"quick br"` finds `quick brown`).
  - `Query::all_of` is `AND`; `Query::except` is `NOT`; `Query::leaves` numbers the phrases `Instance::phrase` refers to.
  - Held to FTS5 by `tests/fulltext_vs_fts5.rs`: prefix, `AND` and `(a OR b) NOT c` match FTS5 document for document, order for order and score for score.
- **Added:** `tests/task_manager_recipes.rs` and `tests/change_feed_recipe.rs`, which pin the supported recipes for several filters (index the list, filter the rest), `(list, due)` range keys, single-record drag-and-drop reorder, and a `seq`-stamped change feed with tombstones.
- **Added:** `rusty_multimodal_db` ADR-0125 (accepted): the change feed stays app-side.
- **Changed:** `Ordered`'s docs state that only the outermost layer answers `PageBy`/`RangeBy` and that inner orders go through `inner()`. Forwarding them is a trait-coherence error (E0119), so this is a documented limit, not a fix.
- Known limits:
  - Prefix matching scans the vocabulary (fine for tasks and notes).
  - FTS5 drops some phrase instances when scoring `(a AND b) NOT c`, so that nesting is pinned for matching documents only, not scores.
  - Per-column filters are not added.

---

## rusty_tick: storage spike and HTTP API
**2026-09-29** · Issue [#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)

- **Added:** `crates/apps/rusty_tick` (`layer = "apps"`): a `TaskStore` over `rusty_multimodal_db_engine` and probes for the gaps #382 lists. Findings are in its `SPIKE-FINDINGS.md`.
- **Added:** a JSON HTTP API (`rusty_tick` binary): lists, tasks, subtasks, tags, manual ordering, search and Today/Next 7 Days/Overdue smart lists. Built on `rusty_http`, `rusty_json` and `rusty_url` with a sans-IO router and blocking std sockets (ADR-0001); bearer-token auth, bounded head/body/idle/connection limits, loopback-only unless `--allow-remote`.
- **Known limitations:** no web UI, sync, multi-user, recurrence or reminders; plain HTTP only; search is whole-word (engine has no prefix query, #382). Scale numbers are one run on one machine.

---

## rusty_multimodal_db_engine: group commit
**2026-09-25** · `rusty_remind_me` ADR-0021 phase 3

- **Added:** `GroupCommit`.
  - `defer_sync()` makes each write return once its insert-log entry is written, before it is synced.
  - `commit()` syncs every such entry and goes back to syncing each write.
  - Reads see deferred writes at once. They are durable only once `commit` returns `Ok`.
  - Implemented by `GenericMmapStore`, and forwarded by the layers with no files of their own: `Indexed`, `Scanned`, `NameIndex` and `Ordered`.
- `insert_log` gains `LogSync`, `append_record`, `append_tombstone_as` and `sync`, and `GenericMmapStore` gains `is_sync_deferred`. Existing appends still sync each entry.
- New integration test `tests/group_commit.rs` covers:
  - a batch of inserts, replaces and deletes seen at once and folded on reopen;
  - a commit with nothing written;
  - a compaction inside a batch.
- In the `rusty_remind_me` hub benchmark, 100-record pushes went from 1327 to 17 040 records/s with 4 pushers.
- **Fixed:** `GenericMmapStore` now holds its records boxed (`HashMap<Id, Box<R>>`).
  - The map regrows in one step, and with records inline a regrow copied every record: a 170–370 ms write pause at 115 000 hub memories of ~800 bytes each.
  - Boxed, the pause is under 10 ms at 115 000 rows and 37 ms at 229 000.
  - The field is private, so the API is unchanged.

---

## rusty_multimodal_db_engine: inserts no longer read the whole insert log
**2026-09-25** · `rusty_remind_me` ADR-0021 phase 3

- **Fixed:** `insert_log::on_disk_version` read the whole insert log on every append to get four header bytes. Every insert and replace therefore got slower as the log grew, until the next `open` or `compact()` folded it. It now reads the header only. A new unit test pins the header-only behaviour, and the engine's and `rusty_multimodal_db`'s suites pass unchanged.
- In `rusty_remind_me`'s hub benchmark (`remind_me_hub/examples/pull_latency.rs`), loading 20 000 memories took 25.1 s before the fix and 5.2 s after. SQLite takes 5.7 s for the same load.

---

## rusty_multimodal_db's generic store extracted to a libs crate
**2026-09-25** · PR [#328](https://github.com/Rusty-Mill/rusty_mill/pull/328) · ADR `rusty_multimodal_db` [`0124`](crates/apps/rusty_multimodal_db/docs/decisions/ADR-0124-engine-extracted-to-libs.md)

Phase 1 of `rusty_remind_me`'s ADR-0021: its hub will embed this store, and
the layer check forbids one apps crate depending on another's.

- **Added:** `crates/libs/storage/rusty_multimodal_db_engine` (`layer = "libs"`):
  the generic store (traits, store layers, `GenericMmapStore`,
  `GenericProductionStore`), `DurabilityError`, the shared blob header and
  `codec`, moved with `git mv` so their history is kept.
- **Changed:** `rusty_multimodal_db` depends on it and re-exports it under the
  paths it had, so its public API is unchanged. `DurabilityError::Store` now
  holds a boxed error; the crate's `From` impls keep the Dog `StoreError`
  round trip. There are no on-disk format or wire changes.

---

## Import rusty_remind_me into crates/apps/rusty_remind_me; first product released from the monorepo
**2026-09-24** · ADR [`0004`](docs/adr/0004-release-products-from-the-monorepo.md)

Ninth merge, same `git subtree` process as ADR-0001's waves (full history,
442 commits). `rusty_remind_me` is the first merged product that ships
releases and a Claude Code plugin, so its distribution moved with it.
Product-level detail is in the crate's own `RELEASE_NOTES.md` (v0.2.1) and
`docs/adr/0020`.

- **Added:** six workspace members under `crates/apps/rusty_remind_me/crates/`
  (`layer = "apps"`), with their intra-family path dependencies hoisted into
  `[workspace.dependencies]`.
- **Added:** `.github/workflows/remind-me-release.yml` (tags
  `rusty-remind-me-vX.Y.Z`, `make_latest: false`) and
  `.github/workflows/remind-me-checks.yml` (schema drift against
  `baileyrd/remind_me` on a daily schedule, plugin/crate version lockstep).
- **Added:** `.claude-plugin/marketplace.json` at the repository root
  (marketplace `rusty-mill`), listing the `rusty-remind-me` plugin by
  relative path.
- **Added:** `ci.yml` jobs `remind-me`, `remind-me-features` (8 legs),
  `remind-me-combined-features`, `remind-me-hub` (Postgres service) and
  `remind-me-windows`, gated on a new `remind_me` plan output.
- **Changed:** `select-packages` gained an all-OS `exclude` input. The
  generic clippy/test jobs exclude the six crates, whose `--all-features`
  build would compile whisper.cpp, usearch and the AWS SDK and link
  libunwind's ptrace API, on Windows too.
- **Changed:** `Cargo.lock` gained 87 new packages and a second version of
  14 existing ones (mostly optional-feature trees: the AWS SDK's
  hyper 0.14/rustls 0.21, rten/tokenizers, whisper-rs, usearch). No existing
  entry was removed or changed version.
- **Known limitation:** the subtree merge commit has no `git-subtree-*`
  trailers (git subtree's `-m` replaces its generated message) and is
  unsigned; amending it was blocked in the session that made it. The
  trailers are a lookup shortcut for `git subtree split`, which can still
  reconstruct the split from history, so nothing depends on them.

## ADR-0003 Phase 4: apps/tools layer (125 crates, 21 families) — migration complete
**2026-09-15** · spec [`PHASE-4-SPEC.md`](PHASE-4-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

Sixth and final implementation slice of ADR-0003. Every one of the 239
workspace members now lives under its assigned layer directory
(`foundation/`, `platform/`, `libs/`, `apps/`, `tools/`), matching
Appendix B exactly. Same mixed-authorship pattern as Phases 1-3.

- **Changed:** 125 crates across 21 families moved — 20 to
  `crates/apps/`, 1 (`rusty_boot`) to `crates/tools/`.
  `rusty_inventrory` renamed to `rusty_inventory` during its move.
- **Removed:** `crates/rustils/` and `crates/rustils_async/` no longer
  exist — `coreutils`/`coreutils-async` (deferred there since Phase 1)
  moved out to their own single-crate `apps/` families, completing the
  split.
- **Fixed:** 3 real functional CI lines — `rusty_meshed`'s vendored
  `data-mesh-monitor` job's change-filter, `working-directory`, and
  `cache-dependency-path` would have silently stopped triggering/
  running without this fix.
- **Fixed:** the 7 Nexus test guards deferred since Phase 1
  (`dep_invariants`, `plugin_contract_purity`,
  `tauri_command_boundary`, `bootstrap_coverage`,
  `core_plugin_loc_budget`, `ipc_topic_prefix_invariant`,
  `dep_invariants_shell`) — all already used a robust dynamic
  workspace-root walk from Phase 1's own fix pattern; only their
  hardcoded path-prefix strings needed updating. One separate,
  pre-existing, unrelated bug in `ipc_topic_prefix_invariant.rs` was
  identified and deliberately left untouched (never valid, not
  introduced by this migration).
- **Fixed:** the last `rush`→`rusty_lines` cascade (Phase 3's fix
  needed re-deriving once `rush` itself moved a level deeper this
  phase) and 25 stale comment/doc references across 9 files (including
  `ARCHITECTURE.md` and two crates' own live documentation) — a Codex
  build round's own grep sweep found all of these; the host categorized
  and resolved each rather than guessing.
- **Regenerated** `docs/WORKSPACE-MAP.md`: `coreutils`/
  `coreutils-async` and the 3 renamed `rusty_inventory` crates now show
  their correct family (previously the leftover `rustils`/
  `rusty_inventrory` names from the intermediate migration state).
- **Verified:** dependency graph identical before/after (by package
  name) across all 22 changed files; all 23 Nexus guard tests pass,
  independently re-run by the host (not just Codex's own report).

## ADR-0003 Phase 3: libs layer (66 crates, 34 families)
**2026-09-15** · spec [`PHASE-3-SPEC.md`](PHASE-3-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

Fifth implementation slice of ADR-0003. Same mixed-authorship split as
Phases 1-2, plus a proactive fix and a build round that caught two real
spec gaps before they shipped.

- **Changed:** 66 crates across 34 families moved to `crates/libs/`, 31
  grouped into a theme (`ai`/`async`/`homelab`/`net`/`protocol`/
  `storage`/`ui`), 3 with no theme.
- **Fixed (proactive):** `check_workspace_layers.py`'s `package_family()`
  generalized to skip a `libs/` theme directory as well as a layer
  directory — done *before* this phase's move, closing the same class
  of bug Phase 1 hit reactively, this time before it could cause a CI
  failure.
- **Fixed:** the last of Phase 0b's 6 `default-features`-excluded
  entries, `rush`→`rusty_lines`.
- **Fixed:** two gaps a Codex build round caught and correctly declined
  to guess at — 28 additional `[workspace.dependencies]` path entries
  the host's own sweep missed (sub-crate package names differing from
  their family directory name, e.g. `adk-core`, `rusty-db-core`,
  `rusty-search-core`), and a cross-phase cascade where Phase 2's own
  fix for `rusty_oauth`/`rusty_request`/`rusty_rag` needed re-deriving
  because all three are `libs`-layer crates that moved again in this
  phase, one level deeper than where that fix was computed.
- **Verified:** dependency graph identical before/after (by package
  name) — 1443 names, 0 changed; `cargo metadata` resolves cleanly
  where it previously failed outright on the unfixed cascade; `cargo
  check` succeeds on the cascading-fix crates plus a sample of the
  28-entry fix.

## ADR-0003 Phase 2: foundation layer (30 crates, 26 families)
**2026-09-15** · spec [`PHASE-2-SPEC.md`](PHASE-2-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

Fourth implementation slice of ADR-0003. Same mixed-authorship split as
Phase 1 (host `git mv`, Codex content edits, both inspected).

- **Changed:** 30 crates across 26 families moved to `crates/foundation/`.
- **Removed:** the third stale pre-merge nested `[workspace]` manifest,
  `crates/rusty_serde/Cargo.toml`.
- **Fixed:** 3 `libs`-layer manifests (`rusty_oauth`, `rusty_request`,
  `rusty_rag`) whose literal-path dependency on a now-moved foundation
  crate would otherwise point at a directory that no longer exists there
  — each was one of Phase 0b's 6 `default-features`-excluded entries, so
  this was expected, not a surprise.
- **Fixed:** 5 stale path references in comments (root `Cargo.toml`,
  `.github/workflows/ci.yml`, `crates/rush/src/glob.rs`) caught by the
  acceptance criterion's own grep sweep — Codex correctly declined to
  touch them without authorization; the host reviewed and applied all 5
  directly (one-line path-string edits, zero functional risk).
- **Verified:** dependency graph identical before/after (by package
  name); `generate_workspace_map.py --verify` passes — this was the gap
  that caused Phase 1's first CI failure (`package_family()` derived the
  wrong family for a crate nested under a layer directory), fixed then
  and confirmed holding here; `cargo check` succeeds on the 3
  manually-fixed crates plus a foundation sample.

## ADR-0003 Phase 1: rustils/rustils_async split, rusty_test → portable-runtime
**2026-09-15** · spec [`PHASE-1-SPEC.md`](PHASE-1-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

Third implementation slice of ADR-0003, and the first with actual
directory moves. Mixed authorship: Codex's sandbox can't write
`.git/worktrees/<name>/index.lock` (outside a linked worktree's own
directory tree), so the host performed the `git mv`/`git rm` step
directly and Codex applied the manifest/source/doc edits on top; both
halves inspected by the host.

- **Changed:** 7 `rustils` crates (`platform`, `platform-bsd`,
  `platform-linux`, `platform-mock`, `platform-parity`,
  `platform-windows`, `winargv`) and 5 `rustils_async` crates
  (`platform-async`, `platform-async-linux`, `platform-async-mock`,
  `reactor-core`, `threading`) moved to `crates/platform/rustils{,_async}/
  crates/`. `coreutils`/`coreutils-async` stay at their current path
  until Phase 4 — both already depended on every moved crate via
  `workspace = true` (a direct payoff of Phase 0b's hoisting), so
  neither needed a manifest edit.
- **Changed:** `rusty_test` moved and renamed to `crates/platform/
  portable-runtime/` intact (6 crates, no split).
- **Removed:** two stale pre-merge nested `[workspace]` manifests,
  `crates/rustils/Cargo.toml` and `crates/rustils_async/Cargo.toml`.
- **Fixed:** `conformance`'s `layering.rs` test hardcoded a
  `crates/rusty_test/` prefix and a fixed 4-level `ancestors()` walk to
  find the workspace root — both only correct before this move. Now
  derives its group prefix from the new path and walks up dynamically
  until it finds the `[workspace]` manifest, matching the pattern
  already used by Nexus's own test guards.
- **Verified:** dependency graph identical before/after (compared by
  package name, since node ids embed the manifest path); `cargo test -p
  conformance --test layering` passes from the new location; `cargo
  check` succeeds on a representative sample including the two crates
  left behind; `git log --follow` confirms history survived every
  rename (234 renames at 100% similarity, 2 deletions, nothing else).

## ADR-0003 Phase 0b: hoist cross-family path dependencies
**2026-09-15** · spec [`PHASE-0B-SPEC.md`](PHASE-0B-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

Second implementation slice of ADR-0003. Built by Codex (`/codex-build`)
from a host-derived, data-backed work order; took two build rounds — the
first correctly stopped rather than force a fix through a real Cargo
restriction it hit mid-proof, the host resolved it, the second round
applied cleanly.

- **Added:** 40 new `[workspace.dependencies]` entries at root (12
  targets already had one).
- **Changed:** 241 dependency entries across 83 member manifests switched
  from a relative `path = "..."` to `dep.workspace = true`, matching
  ADR-0003's own methodology (cross-family = would break once its family
  moves in Phases 1-4).
- **Known exception:** 6 entries left as direct `path` dependencies,
  deliberately not hoisted — Cargo forbids `default-features = false` on
  a `workspace = true` dependency unless the workspace-level entry
  itself already disables default features, and every *other* consumer
  of those 5 target crates needs defaults on. Changing the shared root
  default to accommodate one minority consumer would be a real behavior
  change, not a safe mechanical hoist, so those 6 lines stay as they
  were; each is documented with its reason in `PLAN-REVIEW-LOG.md`.
- **Verified:** the full resolved dependency graph (`cargo metadata`,
  diffed node-by-node) is byte-identical before and after, in both
  `--all-features` and plain mode — this PR has zero effect on what
  actually builds.

## ADR-0003 Phase 0a: workspace layer metadata, CI checker, generated map
**2026-09-15** · spec [`PHASE-0A-SPEC.md`](PHASE-0A-SPEC.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

First implementation slice of ADR-0003 (the `crates/` layer
reorganization), scoped to Phase 0a only — metadata and tooling, zero
directory moves — after the owner was asked to pick a scope given the
ADR's own multi-PR migration plan. Built by Codex (`/codex-build`) from a
host-derived work order restating the ADR's "Phase 0a in detail" section;
independently inspected by the host, which does not delegate to Codex
what Codex itself just built.

- **Added:** `[package.metadata.rusty_mill] layer` on all 239 workspace
  members, matching ADR-0003 Appendix B exactly (verified by a direct
  cross-check, not just the new checker's own pass: 0 mismatches, 0
  missing, 0 extras).
- **Added:** `.github/scripts/check_workspace_layers.py` (+ tests) —
  fails CI on a member with no/invalid layer, an edge to a higher layer,
  or an `apps`-layer crate depending on another family's `apps` crate
  (`tools`-layer callers, e.g. `rusty_boot`→`rush`, are exempt by
  construction). Wired into the existing `dependency-policy` job.
- **Added:** `.github/scripts/generate_workspace_map.py` (+ tests) and
  the generated `docs/WORKSPACE-MAP.md` (239 rows: layer, family, crate,
  description, dependents count), with a `--verify` mode wired into the
  same CI job to fail on drift.
- **Changed:** `README.md`'s hand-maintained "How the crates relate"
  narrative (885 lines) replaced with a pointer to the generated map; the
  crate table and "History" section are untouched.
- Known cost, not a defect: because every one of the 239 manifests
  changed, `affected_crates.py` marks the whole workspace as affected, so
  this PR's CI run is the full build/test/clippy/cross-compile matrix,
  not the lightweight subset a docs-only PR gets.

## ADR-0003 revised after independent Codex review
**2026-09-15** · plan [`docs/adr/0003-workspace-layout-by-layer.md`](docs/adr/0003-workspace-layout-by-layer.md) · log [`PLAN-REVIEW-LOG.md`](PLAN-REVIEW-LOG.md)

`/codex-build` review mode ran an independent Codex review of ADR-0003
(the proposed `crates/` layer reorganization from PR #220, which Codex
could not review at drafting time). Took three rounds to reach APPROVED.

- **Changed:** the migration plan's "what each move phase touches"
  section now lists 8 Rust test files that hardcode a crate-group's
  current path and would silently break at move time —
  `crates/rusty_test/crates/conformance/tests/layering.rs` and 7 Nexus
  guards under `crates/nexus/crates/nexus-bootstrap/tests/` — with the
  specific fix for each (round 1 caught 5 of the 8; round 2's re-review
  found 3 more, which the fix now generalizes into a repeatable `rg`
  sweep rather than a fixed list).
- **Changed:** the Phase 0a checker's cross-family dependency rule now
  applies only to `apps`-layer callers; as originally worded it would
  have rejected `rusty_boot`'s existing, plan-permitted `rush`
  dependency (a `tools`-layer crate, which the plan's own layer table
  already permits to depend on everything).
- Still **Proposed**; this round changed the plan text only, nothing
  implemented or moved.

## Round 7 `/codex-build` monorepo review: 38 findings fixed
**2026-09-13** · report [`CODEX-MONOREPO-REVIEW-2026-09-13-round7.md`](CODEX-MONOREPO-REVIEW-2026-09-13-round7.md)

Seventh independent review pass in this recurring series (rounds 1-6:
`CODEX-MONOREPO-REVIEW.md`, `-2026-09-12.md`, `-round3.md` through
`-round6.md`). 15 parallel read-only scouts covered crate families no
prior round had reached at all or only shallowly: `rusty_git`, `rusty_diff`,
`rusty_ansi`/`mill-term`/`rpath`/`rusty_term`'s `l13`, `rusty_serde`,
`rusty_tls`/`rusty_wiremock`, `rusty_libc`, `rusty_regx`/`rusty_simd`/
`rusty_sha1`/`rusty_text`, `rusty_crypto_key`/`rusty_gpu`/`rusty_vulkan`,
`rusty_provider`'s adapters and `rusty_inventrory`'s `inventory-core`,
`rusty_llama` beyond earlier GGUF-only fixes, and roughly two dozen leaf
`nexus-*` crates never previously audited in depth.

- **Fixed:** 38 findings, each landed with a regression test proven to
  fail against the pre-fix code and pass post-fix, by 25 parallel fix
  tasks. Full per-finding detail (location, trigger, fix, regression test,
  severity) is in the linked report; see its own **Disposition** table and
  per-crate sections rather than duplicating that detail here.
- **Fixed:** 7 additional real lint violations (`clippy::doc_markdown`,
  `doc_lazy_continuation`, `unnecessary_cast`, `collapsible_match`,
  `cast_possible_wrap` ×3, `redundant_guards`) introduced by the fix code
  itself, caught by a closing `cargo fmt` + `cargo clippy --all-targets -D
  warnings` sweep across all 25 touched crates and fixed directly.
- **Known limitation, flagged for a future round, not fixed here:**
  `rusty_serde`'s `Value` type has no custom `Drop`, so a sufficiently
  deep in-memory `Value` tree (~2,000-5,000+ levels) overflows the stack
  merely by being dropped, independently of the deserializer recursion
  guards this round added (which prevent such a tree from being built by
  the JSON/RON text parsers in the first place, but don't protect a
  `Value` tree constructed by hand via the public API).

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Twitter extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-twitter`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-twitter)

Twenty-ninth Phase 1 increment. The twentieth and last of the 20
built-in extractors — Phase 1's extractor set is now complete.

- **Added, core capability:** `Document::extra_documents`/
  `Document::skip_indexing` on `rusty-hister-core` — an additive
  extension (empty/`false` by default, every prior extractor unaffected)
  mirroring Go's own `Document.ExtraDocuments`/`SkipIndexing` shape
  directly rather than growing `ExtractOutcome` a new variant, so no
  signature changes were needed anywhere else in the SDK. Resolves
  `PROJECT-STATUS.md`'s "per-page multi-document extraction" open item
  and unblocks Mastodon/Bluesky/Twitter.
- **Added, extractor:** `TwitterExtractor` (capability inventory §4.5.15)
  — a port of `server/extractor/extractors/twitter/extractor.go`.
  Decomposes a Twitter/X profile/feed/tweet page into one `Document` per
  visible tweet, the third and last extractor to use the capability
  extension above.
- **One source, deduped by URL:** unlike Bluesky's own multi-source
  merge, Twitter has only one real source (the rendered DOM) plus a
  page-meta fallback, and candidates are deduped by canonical URL (first
  match wins) rather than merged field-by-field.
- **`t.co` link unshortening:** Twitter/X shortens every link in a
  tweet's body through its own `t.co` redirector, so this port rewrites
  each `t.co` anchor's `href` back to its real destination (from a
  `data-expanded-url`/`title` attribute or the anchor's own visible
  text) and patches the plain-text version the same way — reparsing the
  whole tweet subtree as its own fragment up front so both that rewrite
  and the ordinary relative-to-absolute URL pass can mutate the same
  copy in sequence. One Go behavior isn't reproduced: replacing a
  `t.co`-only anchor's *visible text* with the expanded URL — cosmetic
  only, since the more important `href` fix and the plain-text
  substitution both still happen.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Bluesky extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-bluesky`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-bluesky)

Twenty-eighth Phase 1 increment. The nineteenth of the 20 built-in
extractors.

- **Added, core capability:** `Document::extra_documents`/
  `Document::skip_indexing` on `rusty-hister-core` — an additive
  extension (empty/`false` by default, every prior extractor unaffected)
  mirroring Go's own `Document.ExtraDocuments`/`SkipIndexing` shape
  directly rather than growing `ExtractOutcome` a new variant, so no
  signature changes were needed anywhere else in the SDK. Resolves
  `PROJECT-STATUS.md`'s "per-page multi-document extraction" open item
  and unblocks Mastodon/Bluesky/Twitter.
- **Added, extractor:** `BlueskyExtractor` (capability inventory §4.5.14)
  — a port of `server/extractor/extractors/bluesky/extractor.go`.
  Decomposes a Bluesky profile/feed/thread page into one `Document` per
  visible post, the first extractor to use the capability extension
  above.
- **Three sources, one merge:** Bluesky pages can carry the same post
  data in up to three independent forms; like Go, this port tries all
  three in priority order and merges results by canonical post URL
  rather than picking just one: a `schema.org` JSON-LD block walked
  recursively through several wrapper keys, the already-rendered post
  DOM (found via known selectors plus a fallback heuristic that walks up
  from any anchor linking to a post URL), and — only when neither of
  those finds anything — the page's own Open Graph/Twitter Card meta
  tags as a single fallback post for the page's own URL.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Mastodon extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-mastodon`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-mastodon)

Twenty-seventh Phase 1 increment. The eighteenth of the 20 built-in
extractors.

- **Added, core capability:** `Document::extra_documents`/
  `Document::skip_indexing` on `rusty-hister-core` — an additive
  extension (empty/`false` by default, every prior extractor unaffected)
  mirroring Go's own `Document.ExtraDocuments`/`SkipIndexing` shape
  directly rather than growing `ExtractOutcome` a new variant, so no
  signature changes were needed anywhere else in the SDK. Resolves
  `PROJECT-STATUS.md`'s "per-page multi-document extraction" open item
  and unblocks Mastodon/Bluesky/Twitter.
- **Added, extractor:** `MastodonExtractor` (capability inventory
  §4.5.13) — a port of
  `server/extractor/extractors/mastodon/extractor.go`. Decomposes a
  Mastodon timeline/status page into one `Document` per visible toot,
  the first extractor to use the capability extension above.
- **Recursion guard:** `matches` accepts a real Mastodon page
  (fingerprinted the same way Go does) or a toot document this extractor
  already produced (`metadata["type"] == "toot"`, matching Go's own),
  so a future indexer re-running the chain over each `extra_documents`
  entry doesn't re-explode an already-extracted toot.
- **Faithful, not "fixed":** `preview` is a direct port of Go's own
  admittedly unfinished implementation (Go's source itself carries a
  `// TODO enhance the toot preview` comment): an optional
  `<h1>`-derived heading followed by the *entire original page's* raw
  HTML, sanitized.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Readability extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-readability`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-readability)

Twenty-sixth Phase 1 increment. The seventeenth of the 20 built-in extractors.

- **Added:** `ReadabilityExtractor` (capability inventory §4.4) — a port
  of the `readabilityExtractor` in `server/extractor/extractor.go`.
  Extract and preview for any web page via the `readabilityrs` crate (a
  Rust port of Mozilla's Readability.js — the same algorithm family Go's
  own `go-readability` dependency belongs to), stripping navigation, ads,
  and other boilerplate down to the main article content. `matches`
  always returns `true`, like `BasicExtractor`; the real chain places
  this extractor right before `Basic`.
- **Dependency decision, by explicit user choice:** `readabilityrs`
  pulls in `scraper 0.25`/`ego-tree 0.10`, a major version ahead of this
  crate's own `0.21`/`0.9` pins used by 8 existing extractors. Both
  versions now coexist in the dependency tree (extra compile time/disk,
  zero risk to the existing extractors) rather than bumping the existing
  pin or hand-rolling the algorithm.
- **Error-shape adaptation:** Go's `readability.FromReader` folds URL
  validation and article extraction into one error path; `readabilityrs`
  splits them, so this port maps a URL-parse failure to `Abort` (matching
  Go's own separate `url.Parse` failure) and a malformed-HTML error or
  no-article-found result to `Fallback` (matching Go's `FromReader` error
  path).
- **Honest gaps:** two Go metadata fields (a favicon URL, a `modified`
  timestamp) have no `readabilityrs` equivalent and are deliberately not
  reproduced rather than silently dropped.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Notion extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-notion`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-notion)

Twenty-fifth Phase 1 increment. The sixteenth of the 20 built-in extractors.

- **Added:** `NotionExtractor` (capability inventory §4.5.16) — a port of
  `server/extractor/extractors/notion/notion.go`. Extract and preview for
  Notion pages on `notion.so` and `*.notion.site`.
- **Unblocked, not blocked:** previously flagged as waiting on the
  JS-rendering crawler backend; re-reading Go's source shows the
  extractor code itself only ever reads `document.html`, like every
  other extractor here, so the crawler dependency is a *production* one
  (whether that field holds real rendered content, since Notion serves
  an empty SPA shell over plain HTTP), not a code dependency. Reports
  `Abort` rather than `Fallback` when the rendered block tree isn't
  present, matching Go's own `AbortExtraction`/`AbortPreview`.
- **One selector, any depth:** Notion's rendered DOM nests presentational
  wrapper `<div>`s deeply; a single substring-attribute selector
  (`[class*="notion-"][class*="-block"]`, identical to Go's own
  `goquery` selector) finds every block at any depth, so both the text
  and HTML walks skip a match whose own ancestor also matches to avoid
  double-counting a block's children.
- **A faithful lossy quirk:** list/heading/quote/paragraph blocks render
  via plain-text extraction before escaping, exactly like Go's own
  `writeTag`/list handling — so an inline `<a href>` inside one of those
  is flattened to text, not preserved as a link; only the image block's
  `src` attribute is read directly and thus round-trips through URL
  rewriting.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Markdown and Org extractors
**2026-09-13** · branch [`claude/hister-phase1-extractor-markdown-org`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-markdown-org)

Twenty-fourth Phase 1 increment. The fourteenth and fifteenth of the 20
built-in extractors.

- **Added:** `MarkdownExtractor` (capability inventory §4.5.1) and
  `OrgModeExtractor` (§4.5.2) — ports of
  `server/extractor/extractors/{markdown/markdown,org/org}.go`.
  Preview-only, structurally identical twins for locally indexed
  Markdown/Org files.
- **No new dependency needed, correcting an earlier assumption:**
  `Indexer.AddMarkdown`/`AddOrg` (capability inventory §5.7,
  `rusty-hister-indexer`'s future job, not this crate's) already renders
  the source to HTML and stores it in `document.html` at index time, so
  each extractor's only job is to sanitize and return whatever HTML is
  already there — no markdown/org-mode parser belongs in this crate at
  all.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Ytdlp extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-ytdlp`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-ytdlp)

Twenty-third Phase 1 increment. The thirteenth of the 20 built-in extractors.

- **Added:** `YtdlpExtractor` (capability inventory §4.5.17) — a port of
  `server/extractor/extractors/ytdlp/{ytdlp,types,format,vtt}.go`.
  Extract and preview for video-hosting pages (YouTube, Vimeo, and
  others), by shelling out to the external `yt-dlp` binary rather than
  parsing `document.html` at all — the only extractor in this crate that
  works entirely from `document.url`. Disabled by default, matching Go,
  since it's useless without `yt-dlp` installed.
- **Three deliberate simplifications** from the Go original, documented
  rather than worked around: no thumbnail download (no general-purpose
  HTTP client in this cluster to reuse — `rusty_http` is a sans-IO
  protocol layer with no client; stores `thumbnail_url` instead of Go's
  base64-embedded image data), no per-instance job-slot concurrency limit
  or cancellation, and preview renders HTML directly rather than Go's
  structured JSON handed to a frontend template.
- **New trick:** the first extractor to use `rusty_json`'s `serde`
  feature (`#[derive(serde::Deserialize)]`) rather than walking
  `rusty_json::Value` by hand, since `yt-dlp --dump-json`'s output is a
  fixed, known shape.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Discourse extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-discourse`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-discourse)

Twenty-second Phase 1 increment. The twelfth of the 20 built-in extractors.

- **Added:** `DiscourseExtractor` (capability inventory §4.5.4) — a port of
  `server/extractor/extractors/discourse/discourse.go`. Extract and
  preview for Discourse forum topic pages.
- **Three sources, one merge:** like Reddit, a topic page can carry the
  same content in up to three places at once — a (often
  double-JSON-encoded) `#data-preloaded` hydration blob, the
  already-rendered post DOM, and a `schema.org` `QAPage` JSON-LD block.
  Like Go, this port merges all three by post id/number rather than
  picking just one, preferring each field's highest-fidelity source by a
  `source_rank` (rendered DOM > preloaded JSON > JSON-LD, matching Go's
  own ranking).
- **Reused, not reinvented:** `WikipediaExtractor`'s reparse-as-fragment
  trick for cleaning/URL-rewriting a post body.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Reddit extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-reddit`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-reddit)

Twenty-first Phase 1 increment. The eleventh of the 20 built-in extractors.

- **Added:** `RedditExtractor` (capability inventory §4.5.6) — a port of
  `server/extractor/extractors/reddit/reddit.go`. Extract and preview for
  Reddit post pages.
- **New approach, three markups at once:** Reddit has shipped at least
  three different markups for the same post over the years — modern
  `shreddit-*` web components, the legacy `old.reddit.com` DOM, and a
  `schema.org` JSON-LD block many pages embed regardless of which HTML
  renders. Like Go, this port copes with an ordered list of CSS-selector
  candidates (first non-empty/first-match wins) rather than branching on
  "which Reddit era is this" up front.
- **Reused, not reinvented:** the crate's third real `textutil` caller
  (Go itself shares it across `hackernews`/`discourse`/`reddit`). Two
  tricks already established for `scraper::ElementRef`'s read-only API
  carry over directly: a post/comment body's URL rewriting re-parses that
  subtree's own HTML as a standalone fragment
  (`WikipediaExtractor::extract`'s clone trick), and reading a comment's
  own text when it has no dedicated body element copies only the kept
  nodes into a fresh `ego_tree` fragment (`ChatGptExtractor`'s
  content-cleaning approach) so a nested reply's text isn't
  double-counted into its parent's.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Wikipedia extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-wikipedia`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-wikipedia)

Twentieth Phase 1 increment. The tenth of the 20 built-in extractors, and
the largest port so far.

- **Added:** `WikipediaExtractor` (capability inventory §4.5.12) — a port
  of `server/extractor/extractors/wikipedia/{wikipedia,style,text}.go`.
  Extract and preview for `*.wikipedia.org/wiki/...` article pages:
  article text, infobox key/value pairs, and wikitables for extraction; a
  richly styled preview (inline styles standing in for Wikipedia's own,
  sanitizer-stripped CSS classes) for rendering.
- **New approach, no DOM mutation API to lean on:** Go's `goquery`
  mutates its parse tree in place (`.Remove()`, `.SetAttr()`,
  `.ReplaceWithHtml()`), which `scraper::ElementRef` has no equivalent
  for — it's a read-only view. This port instead mutates the same
  `ego_tree` by `NodeId`: attribute changes via `Tree::get_mut`, removals
  via `NodeMut::detach`, an element swap for the `<video>`-to-`<img>`
  poster replacement. Every pass collects the `NodeId`s a selector needs
  to touch into an owned `Vec` before mutating, since an `ElementRef`
  can't stay borrowed across a `get_mut` call.
- **New dependency:** `html5ever` (already pinned transitively by
  `scraper` at this same version), needed to construct attribute
  names/values by hand for the mutation above.
- **Behavior preserved deliberately:** the infobox text pass reads from
  the original, un-cleaned content while the general article-text pass
  reads from a cleaned clone with navboxes/references/etc. already
  removed — Go builds that clone by re-serializing and re-parsing the
  content subtree, and this port does the same rather than eliminating
  the clone now that in-place mutation is possible.
- **Not reproduced, documented instead:** wrapping a wikitable in a
  horizontally-scrolling `<div>` for preview has no cheap `NodeId`-based
  equivalent and isn't covered by Go's own tests — a cosmetic-only gap,
  not a content or search-quality loss.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Basic extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-basic`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-basic)

Nineteenth Phase 1 increment. The ninth of the 20 built-in extractors.

- **Added:** `BasicExtractor` (capability inventory §4.4) — a port of the
  `basicExtractor` in `server/extractor/extractor.go`. The universal
  last-resort fallback: strips markup from any HTML document and keeps
  whatever plain text and `<title>` remain. `matches` always returns
  `true` — this only works because a real chain places it last, after
  everything more specific has had its chance.
- **Approach, documented rather than perfectly replicated:** Go walks the
  raw byte stream with its own HTML tokenizer, tracking "inside
  `<body>`"/"inside `<script>`/`<style>`/`<noscript>`" by hand; this port
  instead selects the parsed `<body>` element via `scraper` and walks its
  subtree. Simpler, but not quite equivalent for a fragment with no
  `<body>` tag at all — Go would find no text there, while `html5ever`
  always synthesizes one — a difference documented in the module rather
  than reproduced, since it has no practical effect on real crawled pages.
- **Behavior preserved deliberately:** text nodes are concatenated with no
  separators at all (not even between block elements), deliberately
  cruder than `textutil`'s block-aware flattening, matching Go's own
  token-by-token concatenation exactly. `Preview` doesn't derive anything
  from `document.html`; like Go, it just HTML-escapes whatever
  `document.text` already holds, succeeding with empty content when there
  is none rather than falling back.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's ChatGPT extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-chatgpt`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-chatgpt)

Eighteenth Phase 1 increment. The eighth of the 20 built-in extractors.

- **Added:** `ChatGptExtractor` (capability inventory §4.5.18) — a port of
  `server/extractor/extractors/chatgpt/extractor.go`. Extract and preview
  for chatgpt.com conversation URLs (authenticated, public-shared, and
  custom-GPT). Conversation turns are found via `<article
  data-testid="conversation-turn-...">` wrappers, falling back to bare
  `[data-message-author-role]` elements for public-share/custom-GPT pages
  that skip the wrapper; hidden turns/ancestors and cross-role nested
  content are excluded throughout.
- **New approach, no DOM mutation:** `scraper::ElementRef` is read-only,
  so Go's clone-then-remove content-cleaning pattern has no direct
  equivalent. This port instead copies only the kept nodes into a fresh
  `ego_tree`-backed fragment (`Html::new_fragment()` +
  `NodeMut::append()`), used for both plain-text extraction and preview
  HTML.
- **Not generalized speculatively:** Go's own conversation-text writer is
  a superset of `textutil` (it also handles list bullets and table-cell
  separators) and isn't built on `textutil` either, so this port mirrors
  that with its own `ConversationTextWriter` rather than generalizing
  `textutil` for a shape only this one extractor needs — reusing only its
  final `normalize_text` whitespace pass.
- **Behavior preserved deliberately:** the first extractor to report
  `ExtractOutcome`/`PreviewOutcome::Abort` (a matched conversation URL
  with no visible turns) rather than `Fallback`, matching Go's own
  `AbortExtraction` since that's a dead end for the whole chain, not a
  case for the next extractor to try.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's GitHub extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-github`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-github)

Seventeenth Phase 1 increment. The seventh of the 20 built-in extractors.

- **Added:** `GitHubExtractor` (capability inventory §4.5.9) — a port of
  `server/extractor/extractors/github/github.go`. Extract and preview for
  repository overview, issue, issue-list, and pull-request pages on
  github.com. Go matches these four URL shapes with independent regexes;
  hand-rolled here as small string/character-class predicates instead of
  adding a `regex` dependency for what are fairly mechanical path-shape
  checks. Two Go regex quirks are reproduced exactly rather than
  "corrected": the issue-URL pattern allows only a single non-slash
  character after a `#` fragment marker (almost certainly meant to be
  `[^/]*`), while the pull-request pattern allows a full non-slash run.
- **No new dependency:** the README HTML comes from an embedded
  `<script type="application/json">` payload, parsed with `rusty_json`
  (already this crate's dependency for `Metadata`) rather than adding
  `serde_json`.
- **Behavior preserved deliberately:** `Preview` doesn't re-sanitize its
  whole accumulated buffer the way StackExchange/Lobsters/HackerNews do —
  only the README HTML passes through `sanitizer::sanitize_html`,
  matching a real Go asymmetry (the surrounding metadata card is built
  entirely from HTML-escaped plain strings, already safe without a
  second pass).
- **Flagged, not silently dropped:** Mastodon, Bluesky, and Twitter
  (capability inventory §4.5.13-15) each decompose one timeline/thread
  page into multiple indexed documents (Go: `Document.ExtraDocuments`/
  `SkipIndexing`), a capability `rusty-hister-core`'s `Document`/
  `ExtractOutcome` don't model yet. Recorded as an open item in
  PROJECT-STATUS.md rather than worked around with a lossy
  single-document approximation.

Test plan: `cargo test -p rusty-hister-extractor` — 106 passed (8 new
for this increment), 0 failed; `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` both clean.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's HackerNews extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-hackernews`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-hackernews)

Sixteenth Phase 1 increment. The sixth of the 20 built-in extractors, and
the first with a flat, depth-indented comment table rather than nested
markup.

- **Added:** `HackerNewsExtractor` (capability inventory §4.5.11) — a
  port of `server/extractor/extractors/hackernews/hackernews.go`. Extract
  and preview: submission metadata, self text, and the full comment tree
  from a news.ycombinator.com item page. Unlike Lobsters, comments here
  are a flat table with each row's depth carried by an `indent` attribute
  on its leading `td.ind` cell, reconstructed into nested `<ul>`/`<li>`
  lists by tracking that number across the row sequence — a small state
  machine, not recursion.
- **Added:** a new shared `textutil` module — a Rust port of
  `server/extractor/textutil/textutil.go`, which flattens an HTML subtree
  to plain text while turning block-element boundaries and `<br>` into
  line breaks, so multi-paragraph comment bodies don't run together the
  way a bare text-node concatenation would. Go itself shares this helper
  across `hackernews`/`discourse`/`reddit`, so it's already positioned
  for reuse when those extractors land. `ego-tree` (already pinned
  transitively by `scraper` at the same version) is now a direct
  dependency too, needed to name `NodeRef` in `textutil`'s recursive
  tree walk.

Test plan: `cargo test -p rusty-hister-extractor` — 98 passed (8 new for
this increment), 0 failed; `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` both clean.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's Lobsters extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-lobsters`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-lobsters)

Fifteenth Phase 1 increment. The fifth of the 20 built-in extractors, and
the first with a genuinely recursive comment tree.

- **Added:** `LobstersExtractor` (capability inventory §4.5.10) — a port
  of `server/extractor/extractors/lobsters/lobsters.go`. Extract and
  preview: submission metadata, story body, and the full
  recursively-nested comment tree from a lobste.rs story page
  (`li.comments_subtree` nests `ol.comments > li.comments_subtree`
  arbitrarily deep). Walked with `scraper`'s `ElementRef::child_elements()`
  (direct children only, avoiding the double-visit a broader descendant
  selector would cause on nested subtrees), mirroring the shape of Go's
  own recursive helpers.
- **Reuse, not duplication:** `StackExchangeExtractor`'s small
  selector/text/escaping helpers (`selector`, `element_text`,
  `html_escape`) are now `pub(crate)` and reused here rather than copied
  a third time.
- **Behavior preserved deliberately:** Go's `writeCommentHTML`
  interpolates comment author/score/timestamp into the accumulated HTML
  *unescaped* (unlike the story header/byline, which does escape). Kept
  as-is rather than "fixed", since the whole accumulated string still
  passes through `sanitizer::sanitize_html` before being returned — the
  same defense Go's own `sanitizer.SanitizeHTML` provides at the same
  point, so the asymmetry has no observable security effect.

Test plan: `cargo test -p rusty-hister-extractor` — 83 passed (6 new for
this increment), 0 failed; `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` both clean.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's GoDoc extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-godoc`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-godoc)

Fourteenth Phase 1 increment. The fourth of the 20 built-in extractors —
preview-only, and the first whose Go implementation needed a hand-rolled
tokenizer/depth-tracker that `scraper`'s DOM-based API replaces outright.

- **Added:** `GoDocExtractor` (capability inventory §4.5.8) — a port of
  `server/extractor/extractors/godoc/godoc.go`. Renders pkg.go.dev's
  `div.Documentation-content` element with its `href`/`src` attributes
  resolved to absolute URLs, via `sanitizer`/`urlutil` from the previous
  increment. Go finds that element with a hand-rolled tokenizer that
  reconstructs HTML byte-by-byte while tracking tag-nesting depth
  (`golang.org/x/net/html` has no CSS-selector API); since `scraper`
  already builds a full DOM, the Rust port collapses this to a single
  CSS class selector plus `ElementRef::html()` to serialize the matched
  subtree — no manual depth-tracking needed.
- **Behavior preserved deliberately:** when no matching element is
  present, Go's tokenizer loop reaches end-of-input having never entered
  the "in article" state, and `Preview` *succeeds* with empty content
  rather than falling back. Reproduced faithfully rather than
  "corrected" to a `Fallback`, since that's this extractor's real,
  observable behavior in Go.

Test plan: `cargo test -p rusty-hister-extractor` — 77 passed (8 new for
this increment), 0 failed; `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` both clean.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's StackExchange extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-stackexchange`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-stackexchange)

Thirteenth Phase 1 increment. The third of the 20 built-in extractors, and
the first with real rendered-HTML preview output rather than an enrich-only
fallback.

- **Added:** `StackExchangeExtractor` (capability inventory §4.5.7) — a
  port of `server/extractor/extractors/stackexchange/stackexchange.go`.
  Extract and preview: pulls the question and every already-rendered
  answer from a Stack Exchange network question page (Stack Overflow,
  Server Fault, Super User, Ask Ubuntu, `*.stackexchange.com`, and a
  handful more), marking the accepted answer and harvesting
  author/tags/score/published/answer-count metadata.
- **Added:** two shared support modules other preview-capable extractors
  will reuse — `sanitizer` (a Rust port of `server/sanitizer/sanitizer.go`
  on `ammonia`, replacing Go's `bluemonday`; its SVG attribute-value
  allow-list is reproduced with hand-rolled character-class predicates
  via `ammonia`'s `attribute_filter` callback rather than adding a
  `regex` dependency, since nothing else in this crate needs general
  regex support) and `urlutil` (a port of
  `server/extractor/urlutil/urlutil.go`'s relative-to-absolute URL
  rewriting, implemented as a whole-document pass via `scraper`'s
  tree-mutation API — broader than Go's per-`goquery.Selection` scoping,
  but harmless for every current caller since each one serializes only
  the specific subtree it cares about afterward).
- **Unchanged scope:** Markdown/Org-mode parsing (§4.5.1-2) still need
  their own, separately-decided dependency — not conflated with this
  increment's HTML-sanitizing infrastructure.

Test plan: `cargo test -p rusty-hister-extractor` — 69 passed (31 new for
this increment), 0 failed; `cargo fmt --check` and `cargo clippy --all-targets
-- -D warnings` both clean.

---

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's EmbeddedVideo extractor
**2026-09-13** · branch [`claude/hister-phase1-extractor-embeddedvideo`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-embeddedvideo)

Twelfth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry and `JsonLdExtractor`, and `rusty-hister-model`'s complete query
layer, previous entries below). The second of the 20 built-in extractors,
and the first to use `scraper`/`ammonia` — the HTML-parsing/sanitizing
dependency choice the previous increment deliberately deferred.

- **Decided:** the HTML-parsing/sanitizing dependency question for the
  extractor cluster — `scraper` (CSS-selector HTML parsing, built on
  `html5ever`) and `ammonia` (HTML sanitizing, also `html5ever`-based),
  added directly to `rusty-hister-extractor`. Both are common,
  well-maintained crates.io crates sharing the same underlying parser, so
  only one HTML-parsing engine enters the dependency tree. A
  sovereignty-loop pass first checked `baileyrd/rusty_dbs` at the user's
  suggestion, but it turned out to be an unrelated private repo (a
  subprocess/editor-integration utility crate, `dbs-connector-support`,
  with `scraper`/`ammonia` added bare and unwrapped) and a poor fit
  regardless — `UNLICENSED`, incompatible with this workspace's `MIT OR
  Apache-2.0`, and `rusty_mill`'s own convention is a full `git subtree`
  merge for a first-party sibling, not a permanent pinned dependency on
  an external repo.
- **Added:** `EmbeddedVideoExtractor` (capability inventory §4.5.3) — a
  port of `server/extractor/extractors/embeddedvideo/extractor.go`.
  Enrich-only: scans `<video>`/`<source>`/`<iframe>`/`<embed>`/`<object>`
  elements via a `scraper` CSS selector (`"video, source, iframe, embed,
  object"`), and stores discovered video URLs — deduplicated, in
  document order — as a JSON array at `Metadata["videos"]`.
  `<iframe>`/`<embed>`/`<object>` URLs are only accepted when they match
  a known video-hosting service by full `https://` prefix (a prefix
  check, not a substring check, so `https://evil.com/youtube.com/embed/`
  is correctly rejected); `<video>`/`<source>` URLs are trusted as
  first-party content, matching Go. `<source>` additionally requires an
  ancestor `<video>` element, reproduced via `scraper`'s ancestor
  traversal in place of Go's token-stream `inVideo` flag.
- **Unchanged scope:** no sanitization needed here — URLs are stored as
  opaque strings, never rendered as HTML, so `ammonia` isn't exercised by
  this extractor (a later one will be the first to need it).
- **Added:** 9 new unit tests (38 total in the crate) — native `<video>`
  with a nested `<source>`, a `<video>`'s own `src` attribute, a
  `<source>` outside any `<video>` correctly ignored, known-host
  acceptance for `<iframe>`, rejection of a URL that only superficially
  resembles a known host, `<embed>`/`<object>` acceptance, URL
  deduplication, the no-video-elements fallback, and the `matches`
  quick-check pre-scan — independently derived, not copied, per
  ADR-0001's licensing policy (this file has no Go test coverage to
  begin with — capability inventory marks it `[UNTESTED]`). clippy/fmt
  clean.

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's first concrete extractor (JSON-LD)
**2026-09-12** · branch [`claude/hister-phase1-extractor-jsonld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-jsonld)

Eleventh Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, and `rusty-hister-model`'s complete query layer, previous
entries below). The first of the 20 built-in extractors (capability
inventory §4.3-§4.5), which have been the crate's only unstarted piece
since the registry itself landed.

- **Added:** `JsonLdExtractor` (capability inventory §4.5.5) — a port of
  `server/extractor/extractors/jsonld/jsonld.go`. Enrich-only: matches
  any document whose HTML contains the `application/ld+json` substring,
  parses every `<script type="application/ld+json">` block, flattens
  `@graph`/array wrappers into a flat node list, deep-sanitizes every
  non-`@`-prefixed string field, stores the sanitized node list as a
  JSON string at `Metadata["jsonld"]`, and writes two classification
  fields — `type` and `headline` — picked from whichever node's `@type`
  best matches a priority list (`Article`, `NewsArticle`, ...,
  `Organization`), falling back to the first node.
- **Unchanged scope:** deliberately doesn't write `author`/`description`/
  `image`/`published`/`modified` — Readability (not yet ported) already
  harvests those from the same JSON-LD data plus OpenGraph/meta tags and
  runs later in the default chain, so writing them here would just be
  overwritten, matching Go's own comment on this.
- **Why JSON-LD first:** chosen specifically because it could reuse this
  cluster's existing `rusty_json` dependency for JSON parsing and needed
  only two small, purpose-built helpers — a `<script>`-block scanner
  (`find_json_ld_blobs`, a tiny state machine, not a DOM parser) and a
  text sanitizer (`sanitize_text`: strip HTML tags, decode a practical
  subset of HTML entities, trim — approximating Go's
  `bluemonday`-backed `sanitizer.SanitizeText` without depending on it)
  — rather than a general HTML-parsing or HTML-sanitizer library. Most
  of the remaining 19 built-in extractors (Discourse, Reddit,
  StackExchange, GitHub, Wikipedia, and others that parse or render real
  page HTML) will need general HTML parsing and/or Go's HTML-vs-text
  sanitizer-policy split for real — a cross-cutting, architecturally
  significant dependency choice flagged in `docs/PROJECT-STATUS.md` for
  explicit sign-off before the next extractor that actually needs it,
  rather than picked unilaterally on this first one.
- **Added:** 11 new unit tests (29 total in the crate) — single-node and
  `@graph`-flattened extraction, multiple `<script>` tags with preferred-
  type ranking, top-level array form, a malformed blob skipped while a
  valid one survives, the `matches` substring pre-check, tag-stripping/
  entity-decoding on the classification fields, and deep sanitization of
  the raw node dump while preserving `@`-prefixed structural keys —
  independently derived from reading `jsonld_test.go`'s behavior, not
  copied, per ADR-0001's licensing policy. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's user.go query layer
**2026-09-12** · branch [`claude/hister-phase1-model-user`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-user)

Tenth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue,
`WebSession`, `DocumentVersion`, `crawl.go`, and `history.go` query
layers, previous entries below). Completes `user.go`'s query layer — the
last of `rusty-hister-model`'s six Go model files, so this crate's query
layer is now fully ported.

- **Added:** `User::{create, create_oauth, delete_by_username,
  authenticate, get_by_token, regenerate_token, get_by_username,
  get_by_id, regenerate_token_by_username, rename, set_password,
  get_by_oauth_id, toggle_admin, rules_json, set_rules_json}` — a port
  of `CreateUser`/`CreateOAuthUser`/`DeleteUser`/`AuthenticateUser`/
  `GetUserByToken`/`RegenerateToken`/`GetUser`/`GetUserByID`/
  `RegenerateTokenByUsername`/`UpdateUsername`/`UpdatePassword`/
  `GetUserByOAuthID`/`ToggleAdmin`/`GetUserRules`/`SaveUserRules`.
- **Added (dependency):** `argon2` — a sovereignty-loop pass found no
  first-party `rusty_*` crate for password hashing, so `create`/
  `set_password` hash with Argon2id via `argon2`, already a workspace
  dependency through `rusty_croc`'s PAKE handshake, rather than
  reimplementing Go's bcrypt or adding a second password-hashing crate.
  Salt bytes come from `rusty_rand` (this crate's established CSPRNG
  entry point), not argon2's own optional `rand` feature, which stays
  disabled. This is a deliberate algorithm change, not a capability
  drop — every password this crate ever hashes is freshly created here
  under the current fresh-install-only working assumption — flagged in
  `docs/PROJECT-STATUS.md` for explicit sign-off.
- **Changed (deliberate simplification, not a dropped capability):**
  `authenticate` collapses Go's two distinct sentinel errors
  (`ErrUserNotFound`/`ErrInvalidPassword`) into one `None` case, verified
  against the only real caller (`server/endpoints.go`'s `serveLogin`),
  which already treats both identically (a bare `err != nil` check → one
  401 response) — avoids a footgun where a future caller could
  accidentally build a username-enumeration oracle from the distinction.
- **Added:** a `RenameOutcome` enum (`Renamed`/`UsernameTaken`/`NotFound`)
  for `rename`'s three possible results, instead of a Go-style sentinel
  error — makes the exhaustive three-way branch checkable by the
  compiler at every call site.
- **Unchanged scope:** Go's `ParseRules`/`config.Rules` (a compiled-regex
  config-rules engine for skip/priority/versioning rules and alias
  expansion) isn't ported — no such engine exists in this Rust codebase
  yet, the same scope boundary `CrawlJob::validator_rules: Json` already
  draws. `rules_json`/`set_rules_json` read/write the stored JSON blob
  as-is; they don't parse or compile it.
- **Unchanged scope:** `regenerate_token`'s no-such-user quirk is
  preserved as-is — Go's version updates zero rows and returns no error
  when `user_id` doesn't exist, always handing back the freshly
  generated token regardless (unlike `regenerate_token_by_username`,
  which does check first, matching Go's own asymmetry between the two).
- **Added:** 25 new unit tests (131 total in the crate) — account
  creation (password and OAuth) including duplicate-username rejection;
  authentication success/wrong-password/unknown-user cases; hard delete
  by username; token issuance/lookup by username and by id (including
  the no-such-user quirk); rename's three outcomes; password change and
  its effect on authentication; OAuth lookup; admin toggling both ways;
  and rules-JSON get/set round-tripping. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's history.go query layer
**2026-09-12** · branch [`claude/hister-phase1-model-history`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-history)

Ninth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue,
`WebSession`, `DocumentVersion`, and `crawl.go` query layers, previous
entries below). Completes `history.go`'s query layer — the tables backing
Hister's search-history tracking (query <-> URL associations) and rows
1.16-1.18's history/pin features.

- **Added:** `Link::get_or_create`/`History::get_or_create` — a port of
  `GetOrCreateLink`/`GetOrCreateHistory`, reusing the database-assigned-
  surrogate-key recipe `WebSession::create` introduced.
- **Added:** `HistoryLink::{delete_by_user_and_url,
  delete_by_user_query_and_url, set_pinned, record_selection,
  urls_by_query, latest_items, timestamps, suggest_query}` — a port of
  `DeleteHistoryURL`/`DeleteHistoryItem`/`SetHistoryPinned`/
  `UpdateHistory`/`GetURLsByQuery`/
  `GetLatestHistoryItems(Filtered(ByDate))`/
  `GetHistoryItemTimestampsFilteredByDate`/`GetQuerySuggestion`.
  `record_selection`, not `update`, avoids colliding with
  `#[derive(Mapped)]`'s own generated `update()` instance method (the
  same reasoning behind `WebSession::refresh`).
- **Changed (deliberate simplification, not a dropped capability):**
  `latest_items` collapses Go's three `GetLatestHistoryItems`/
  `GetLatestHistoryItemsFiltered`/`GetLatestHistoryItemsFilteredByDate`
  wrappers into one function taking a `HistoryItemsFilter` struct — those
  three Go functions are pure argument-forwarding wrappers around the
  same query with different defaults (Go has no default parameters), not
  three different behaviors; every Go call site's arguments map onto a
  field, with `Default` covering what the simpler wrappers omitted.
- **Discovered while porting:** Hister's `CommonFields.DeletedAt` is a
  plain `*time.Time`, not GORM's own `gorm.DeletedAt` sentinel type, so
  GORM never actually recognizes any Hister model as soft-delete-enabled
  — every `DB.Delete(...)` call in `history.go` is a genuine hard delete,
  and the `deleted_at` column is otherwise inert. `delete_by_user_and_url`/
  `delete_by_user_query_and_url` accordingly issue a real `DELETE`
  instead of this crate's `#[table(soft_delete)]` column (which would
  also have broken re-recording history for the same URL afterward,
  since `history_links`' unique index on `(history_id, link_id)` isn't
  scoped to active rows). `get_or_create`'s lookups and the existing-row
  checks in `record_selection`/`set_pinned`'s pin branch still use
  `Mapped::not_deleted_filter()` where Go's own queries are `.Model(&T{})`-
  based — a no-op today since nothing ever sets the column, but correct
  if a real soft-delete path is added later. Flagged in
  `docs/PROJECT-STATUS.md` as an open item: whether the soft-delete
  columns on this schema's other four affected models should be removed
  (closer to Go's actual behavior) or kept as a deliberate Rust-side
  improvement isn't decided yet.
- **Unchanged scope:** `user.go`'s auth/token helpers remain the one
  not-yet-started increment in `rusty-hister-model`'s query layer —
  it'll need its own sovereignty-loop pass for a password-hashing
  approach before implementation starts.
- **Added:** 22 new unit tests (106 total in the crate) — `get_or_create`
  create/reuse/title-update/empty-title-noop behavior for both `Link` and
  `History`; `record_selection`'s count-increment and missing-data
  rejection; `set_pinned`'s create/pin-existing/unpin/missing-data
  behavior; `delete_by_user_and_url`/`delete_by_user_query_and_url`'s
  cross-query vs. single-query scope; `urls_by_query`'s pinned-then-count
  ranking; `latest_items`' newest-first ordering, case-insensitive
  title/URL filtering (with literal SQL-wildcard handling, mirroring
  Hister's own `TestGetLatestHistoryItemsFiltered`), stable
  `(updated_at, id)` keyset pagination (mirroring
  `TestGetLatestHistoryItemsFilteredByDateUsesStableCursor`), and date-range
  filtering; `timestamps`' matching-row count; `suggest_query`'s
  prefix-match ranking and no-match `None`. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's CrawlURL queue mechanics
**2026-09-12** · branch [`claude/hister-phase1-model-crawl-url`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-crawl-url)

Eighth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue,
`WebSession`, `DocumentVersion`, and `CrawlJob`-lifecycle query layers,
previous entries below). Completes `crawl.go`'s query layer: this covers
the 13 `CrawlURL` functions the previous increment deliberately left out
(that file has 20 functions total; 7 operate on `CrawlJob` itself, 13 on
`CrawlURL`).

- **Added:** `CrawlURL::insert_if_not_exists`/`bulk_insert`/
  `mark_done_and_enqueue_links`/`insert_done`/`next_pending`/
  `update_status`/`mark_failed`/`reset_in_progress`/`count_by_status`/
  `count`/`list_failed`/`list`/`job_stats` — a port of
  `InsertCrawlURLIfNotExists`/`BulkInsertCrawlURLs`/
  `MarkCrawlURLDoneAndEnqueueLinks`/`InsertDoneCrawlURL`/
  `GetNextPendingCrawlURL`/`UpdateCrawlURLStatus`/`MarkCrawlURLFailed`/
  `ResetInProgressCrawlURLs`/`CountCrawlURLsByStatus`/`CountCrawlURLs`/
  `ForEachFailedCrawlURL(WithMessage)`/`ForEachCrawlURL(ByStatus)`/
  `GetCrawlJobStats`.
- **Changed:** Go's private `insertCrawlURLs` helper (shared by
  `CreateNamedCrawlJobWithURLs` and `BulkInsertCrawlURLs`) becomes this
  file's own private `insert_crawl_urls`, now reused by
  `CrawlJob::create_with_urls` (refactored to call it) and the new
  `CrawlURL::bulk_insert` alike — a second real call site, same "no
  abstraction before two call sites" discipline as `crate::placeholders`.
- **Added:** a new `CrawlJobStats` plain struct (pending/in_progress/
  done/failed/skipped counts), returned by `job_stats`, which aggregates
  via `SELECT status, COUNT(*) ... GROUP BY status` and decodes each row's
  `status` straight into `CrawlUrlStatus` through `Row::get_by_name`.
- **Changed (deliberate simplification, not a dropped capability):** Go's
  two `ForEach*` streaming-callback iterators (`ForEachFailedCrawlURL(WithMessage)`/
  `ForEachCrawlURL(ByStatus)`) become `list_failed`/`list`, returning a
  `Vec<Self>` instead of taking a callback — this crate has no other
  streaming-query precedent, and nothing yet calls these at a scale where
  collecting matters. Every row Go's callback would see is still
  reachable, just batched.
- **Unchanged scope:** `history.go`'s search/pin/timeline queries and
  `user.go`'s auth/token helpers remain separate, not-yet-started
  increments — `rusty-hister-model`'s query layer now covers four of its
  six domain files (`embedding.go`, `session.go`, `version.go`, `crawl.go`).
- **Added:** 16 new unit tests (81 total in the crate) — dedup on
  `insert_if_not_exists`/`bulk_insert` against already-queued URLs, a
  no-op empty-list `bulk_insert`, `mark_done_and_enqueue_links` updating
  the source row and queuing its child links, `insert_done`'s
  update-or-insert branches, `next_pending`'s oldest-first FIFO order and
  empty-queue `None`, `update_status`/`mark_failed`'s status/error/
  error_code writes, `reset_in_progress` moving only in-progress rows back
  to pending, `count_by_status`/`count`/`list_failed`/`list` reflecting
  the queue accurately, and `job_stats` aggregating correctly including
  the all-zero case for a job with no URLs. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's CrawlJob lifecycle query layer
**2026-09-12** · branch [`claude/hister-phase1-model-crawl-job`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-crawl-job)

Seventh Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue,
`WebSession`, and `DocumentVersion` query layers, previous entries below).
Scoped to `CrawlJob`'s own lifecycle — `crawl.go`'s job-level helpers,
not `CrawlURL`'s queue mechanics, which stay a separate follow-up (that
file has 20 functions total; this increment covers the 7 that operate on
`CrawlJob` itself).

- **Added:** `CrawlJob::generate_id`/`create`/`create_with_urls`/`get`/
  `update_status`/`list`/`delete` — a port of `GenerateCrawlJobID`/
  `CreateCrawlJob`/`CreateNamedCrawlJobWithURLs`/`GetCrawlJob`/
  `UpdateCrawlJobStatus`/`ListCrawlJobs`/`DeleteCrawlJob`.
- **Added:** a new first-party dependency, `rusty_rand` (already existed
  in this workspace as the shared OS-backed-CSPRNG crate extracted from
  three near-identical copies elsewhere) — `generate_id` uses it for
  cryptographically secure random bytes rather than adding the external
  `rand` crate, per this project's sovereignty-loop discipline.
- **Added:** `create_with_urls`'s atomic job-plus-initial-queue creation:
  a `Transaction` that retries the job insert with a `-2`/`-3`/... id
  suffix on collision, then bulk-inserts the initial URLs, deduping
  against any already queued. Both the collision retry and the per-URL
  dedup need `ON CONFLICT DO NOTHING`, which the query builder doesn't
  support, so this drops to raw SQL — the same pattern
  `EmbeddingJob::enqueue` established, extended here to a genuine
  multi-statement transaction via `Engine::begin()`/`Transaction::execute`/
  `commit` rather than a single raw statement.
- **Unchanged scope:** `CrawlURL`'s own queue mechanics (bulk insert,
  per-URL status transitions, the `ForEach*` streaming iterators,
  `GetCrawlJobStats`) are not part of this increment — a separate,
  not-yet-started follow-up.
- **Added:** 9 new unit tests (65 total in the crate) — `generate_id`
  produces distinct 8-character hex strings; `create`/`get`/
  `update_status`/`list`/`delete` round-trip correctly; `create_with_urls`
  rejects an empty URL list, inserts the job and its initial queue, and
  retries with a `-2` suffix when the base id collides. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's DocumentVersion query layer
**2026-09-12** · branch [`claude/hister-phase1-model-document-version`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-document-version)

Sixth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue and
`WebSession` query layers, previous entries below). Scoped to
`DocumentVersion`'s query layer alone — small, self-contained, and no
`CASE`-based SQL needed (unlike the embedding queue), so it's a
straightforward second application of the database-assigned-surrogate-key
recipe `WebSession::create` introduced.

- **Added:** `DocumentVersion::save`/`move_versions`/`count`/`list`/
  `list_until` — a port of `version.go`'s `SaveDocumentVersion`/
  `MoveDocumentVersions`/`CountDocumentVersions`/`GetDocumentVersions`/
  `GetDocumentVersionsUntil`. `save` reuses `WebSession::create`'s raw-
  `INSERT`-plus-`RETURNING`/`last_insert_rowid()` pattern for its
  autoincrementing `id`. `count` is the crate's first aggregate query
  (`Expr::count_all()` via `SelectExpr`).
- **Unchanged scope:** the document-versioning diff format/algorithm
  itself (capability inventory §11) is a separate, still-undecided
  concern — this layer only stores and retrieves whatever `html_diff`/
  `text_diff` text the caller already computed; it doesn't compute diffs.
- **Added:** 6 new unit tests (56 total in the crate) — save assigns an id
  and the row is listed; list filters by both url and user; count reflects
  the stored row count; move_versions reassigns ownership and no-ops for
  the same user; list_until returns only versions at or after the given
  id, newest first. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's WebSession query layer
**2026-09-12** · branch [`claude/hister-phase1-model-websession`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-websession)

Fifth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue query
layer, previous entries below). Scoped to `WebSession`'s query layer
alone — small and fully self-contained, and the first place this crate
needs a database-assigned surrogate key, worth solving and documenting in
isolation before the remaining five model files (which mostly share the
same autoincrementing-`i64`-primary-key shape) need the same recipe.

- **Added:** `WebSession::create`/`get`/`refresh`/`delete` — a port of
  `session.go`'s `CreateWebSession`/`GetWebSession`/`UpdateWebSession`/
  `DeleteWebSession`. `get` returns `Option<WebSession>` rather than a
  `HisterError`/sentinel-error pair for the not-found case — the
  idiomatic Rust equivalent of Go's dedicated `ErrWebSessionNotFound`.
  Named `refresh`, not `update`, since `#[derive(Mapped)]` already
  generates an `update()` instance method (turns a whole struct value
  into an `UPDATE` statement) that a same-named associated function would
  collide with.
- **Added:** `create`'s database-assigned-primary-key recipe —
  `rusty_db::Mapped::insert()` always includes the primary key field's
  current value (there's no "leave this to the database" marker), so it
  can't populate an autoincrementing `i64` column on its own. `create`
  instead issues a raw `INSERT` that omits the `id` column, then recovers
  the generated value dialect-appropriately: `RETURNING id` where
  `Dialect::supports_returning()` is true (Postgres), `SELECT
  last_insert_rowid()` otherwise (SQLite — this crate's dialect model
  reports `false` here even though modern SQLite itself supports
  `RETURNING`). Documented in `session.rs` as the pattern
  `User`/`Link`/`History`/`HistoryLink`/`DocumentVersion` will reuse for
  their own `create`s.
- **Changed:** the dialect-placeholder helper (`Engine::dialect().placeholder(..)`
  rendering) introduced for the embedding queue's raw-SQL functions is
  now a shared `crate::placeholders` in `lib.rs`, since `WebSession::create`
  is a second real call site.
- **Added:** 7 new unit tests (50 total in the crate) — create assigns a
  positive, distinct id per session and the row is retrievable by token
  hash; get returns `None` for an unknown hash; refresh changes
  data/expiry for a known session and reports `false` for an unknown one;
  delete removes a session and no-ops for an unknown one. clippy/fmt
  clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's embedding-queue query layer
**2026-09-12** · branch [`claude/hister-phase1-model-embedding-queue`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-embedding-queue)

Fourth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, and `rusty-hister-model`'s schema, previous entries below). Scoped
to `EmbeddingJob`'s query-layer alone — the first of the six model files'
domain operations flagged as a follow-up when the schema increment landed
— since it's fully self-contained (no dependency on any other table) and
exercises the two genuinely hard parts of this crate's remaining work
(a dialect-portable upsert, and an optimistic-concurrency claim loop) in
isolation before touching anything else.

- **Added:** `EmbeddingJob::enqueue`/`claim_next`/`complete`/`retry`/
  `fail`/`release`/`in_progress_exists`/`delete`/`reset_in_progress` — a
  field-for-field port of `embedding.go`'s durable, deduplicated embedding
  work queue: enqueuing a pending job is a no-op, enqueuing an active job
  marks it dirty instead of resetting it, `claim_next` atomically claims
  the oldest available job (retrying its select-then-claim pair when
  another worker races ahead), and `complete`/`fail` return a dirty job to
  pending instead of deleting/failing it.
- **Added:** a small dialect-portable raw-SQL path for the three
  operations (`enqueue`'s `ON CONFLICT ... DO UPDATE SET` upsert,
  `retry`'s and `release`'s `CASE WHEN ... THEN ... ELSE ... END` SET
  clauses) that `rusty_db`'s query builder can't express — `Update::set`
  only ever takes a `Value`, never an `Expr`. Placeholders are rendered
  per-dialect via `Engine::dialect().placeholder(..)` rather than
  hardcoding `?`/`$N`, so the same SQL text works against both SQLite and
  Postgres; the other six functions use the ordinary `Select`/`Update`/
  `Delete` builder.
- **Known limitation:** the Postgres path is untested — only SQLite is
  exercised (no Postgres instance available in this environment). Flagged
  in `crates/rusty_hister/docs/PROJECT-STATUS.md`'s open items, same risk
  profile as the schema increment's untested `POSTGRES_MIGRATIONS`.
- **Added:** 18 new unit tests (43 total in the crate) — enqueue's
  three outcomes (fresh/idempotent/dirty-marking/failed-job-reset),
  claim_next's ordering and availability filtering, complete/fail's
  dirty-job-returns-to-pending branch, retry's immediate-vs-scheduled
  branch, release's never-negative attempt count, and
  in_progress_exists/delete/reset_in_progress. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's schema
**2026-09-12** · branch [`claude/hister-phase1-model`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model)

Third Phase 1 increment (after `rusty-hister-core` and
`rusty-hister-extractor`'s registry, previous entries below). Scoped to
`rusty-hister-model`'s **schema** — the nine `#[derive(Mapped)]` types and
the migration that creates them — not the domain/query-layer behavior each
Go model file builds on top of its table, which is a separate follow-up
increment (same schema-mechanism-first split `rusty-hister-extractor`'s
registry used).

- **Added:** `User`, `Link`, `History`, `HistoryLink`, `CrawlJob`,
  `CrawlURL`, `WebSession`, `DocumentVersion`, `EmbeddingJob` — Hister's
  nine `automigrate()`-list models (capability inventory §7.2), ported
  field-for-field from `server/model/*.go`. `CrawlJobStatus`/
  `CrawlUrlStatus`/`EmbeddingJobStatus` are `#[derive(MappedEnum)]` closed
  enums rather than Go's untyped string constants, making each field's
  "only these values are valid" invariant checkable by the type system.
- **Added:** soft-delete on the four models that embedded Go's
  `CommonFields` (`User`, `Link`, `History`, `HistoryLink`) via `rusty_db`'s
  first-party `#[table(soft_delete)]`, rather than hand-rolling
  `CommonFields`' nullable `DeletedAt` convention — an equivalent
  capability via a different, already-available mechanism, not a
  simplification.
- **Added:** `SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS` — a fresh-install
  migration (one per backend, since `rusty_db` migrations are plain,
  non-portable SQL) that creates all nine tables and their unique/lookup
  indexes, run through `rusty_db::Migrator`. Hister's own `Database`
  singleton-row schema-version tracker has no Rust equivalent: `Migrator`'s
  own bookkeeping table already solves the same problem, so this is a
  documented substitution, not a dropped capability.
- **Known limitation (by design, flagged for explicit follow-up):** this
  is a fresh-install-only schema. Hister's three historical Go migrations
  (the `history_links.pinned` backfill, the `web_sessions.last_seen_at`
  column drop, the mixed-offset-to-UTC timestamp rewrite) and the legacy
  `indexer_versions` read path are **not** reproduced, since they only
  matter for opening a pre-existing Hister-Go-created database file — a
  new, still-unresolved question (does `rusty_hister` ever need to do
  that at all?) recorded in `crates/rusty_hister/docs/PROJECT-STATUS.md`'s
  open items, alongside the already-flagged `indexer_versions` item it
  subsumes.
- **Added:** 25 unit tests — real round-trips through an in-memory SQLite
  engine (`sqlite::memory:`) for every model, unique-constraint/duplicate-
  rejection checks for every `uniqueIndex` in the Go source
  (`username`, `url`, `(user_id, query)`, `(history_id, link_id)`,
  `(job_id, url)`, `token_hash`), a soft-delete round-trip
  (`Session::delete`/`get`/`load_active`), and migration up/down/status
  checks (including that the schema is actually created and is
  reversible). clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's registry
**2026-09-12** · branch [`claude/hister-phase1-extractor-registry`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-registry)

Second Phase 1 increment (after `rusty-hister-core`, previous entry below).
Scoped to `rusty-hister-extractor`'s `Registry` alone — the
chain-of-responsibility mechanism, not any concrete extractor — since it
only depends on `rusty-hister-core` (already merged) and is a
self-contained, well-specified unit on its own.

- **Added:** `Registry` (capability inventory §4.2): `register`/
  `register_before` with case-insensitive duplicate-name rejection;
  `apply_configs` to merge a pre-parsed name→config map into matching
  extractors (unknown names ignored, matching Hister's own "config for an
  unregistered extractor is a no-op" behavior — parsing an actual config
  *file* into that map is a separate, not-yet-decided concern); `list`/
  `list_enabled`/`list_matching`/`list_matching_preview` introspection.
- **Added:** the two-phase extraction chain — every matching enabled
  enricher runs in chain order first (a `Fallback` is skipped over, only
  `Abort` halts everything), with its enrichment carried forward into the
  next stage; then matching enabled content extractors run in chain order
  until one succeeds or aborts. Verified with a test that actually checks
  the second-phase extractor receives the first phase's enrichment (not
  just that the chain doesn't crash).
- **Added:** the separate preview chain — an optional case-insensitive
  starting-point name skips ahead in chain order without disabling the
  fallback chain after it; a starting point that's unregistered, disabled,
  non-preview-capable, or non-matching is a hard `Abort`, never silently
  ignored.
- **Verified:** 18 unit tests (including every hard-error path and the
  enrichment hand-off), clippy/fmt clean, whole-cluster `cargo check`
  clean, dependency-sovereignty policy clean. `Registry` is deliberately
  not internally synchronized (no mutex) — Hister's Go version guards its
  list because it's shared across concurrent HTTP handlers; that's a
  caller-side concern (e.g. `rusty-hister-server` wrapping it in a
  `Mutex`/`RwLock`), not something to build in speculatively here.
- **Not done here:** no concrete extractors — `rusty-hister-extractor` has
  a working chain mechanism and nothing registered into it yet. That's the
  next increment (capability inventory §4.3-§4.5, in default-chain order).

---

## Start rusty_hister Phase 1: implement rusty-hister-core
**2026-09-12** · branch [`claude/hister-phase1-core`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-core)

First real implementation in the `rusty_hister` cluster (previously all
eight crates were empty skeletons). Scoped to `rusty-hister-core` alone —
the shared contract every other `rusty-hister-*` crate depends on — rather
than all of Phase 1 (`rusty-hister-model`, `-extractor`, `-crawler`'s `http`
backend) in one PR, matching this project's established pattern of
PR-sized increments.

- **Added:** `Document` — the extractor pipeline's working document type
  (url, title, text, html, favicon, label, document type, language,
  metadata), distinct from `rusty-hister-model`'s persisted `History`/`Link`
  rows. `DocumentType` (`Web`/`Local`/`RemoteFile`) deliberately leaves its
  wire-format integer encoding unassigned — capability inventory review
  only confirmed one value ("2 = remote-file snapshot"), and v1's
  byte-compatibility requirement means guessing the rest would risk a wire
  mismatch against real Hister clients; assign it when `rusty-hister-server`
  needs it and the exact values can be confirmed.
- **Added:** the `Extractor` trait (capability inventory §4.1) — `name`,
  `description`, `capabilities`, `matches`, `extract`, `preview`, `config`,
  `set_config` — plus `Capabilities` (independent enrich/extract/preview
  booleans), `ExtractorConfig` (enabled + options bag, enabled by default),
  `PreviewResponse`, and the tri-state `ExtractOutcome`/`PreviewOutcome`
  enums reproducing Hister's `ExtractorSuccess`/`ExtractorFallback`/
  `ExtractorAbort` chain-of-responsibility pattern as a closed Rust enum
  (no opaque-type/factory-function indirection needed — the enum itself
  makes a fourth state unrepresentable). Synchronous by design: every
  extractor operates on an already-fetched `Document`, no I/O to make
  async worth it. The chain-of-responsibility *registry* (§4.2) is left to
  `rusty-hister-extractor`, which will depend on this trait.
- **Added:** `HisterError`, on `rusty_err` (this workspace's own
  `thiserror`+`anyhow` analog) — `InvalidConfig`, `Extraction`, and a
  `BoxError`-backed catch-all, kept deliberately small pending concrete
  failure modes from later phases rather than speculative variants.
- **Verified:** `cargo test -p rusty-hister-core --all-features` (16/16),
  `clippy --all-targets --all-features -- -D warnings`, and `fmt --check`
  all clean; the other seven `rusty-hister-*` skeleton crates still build
  against the new `rusty-hister-core` API; `check_workspace_deps.py`
  (dependency-sovereignty policy) passes.
- **Not done here:** `rusty-hister-model`, `rusty-hister-extractor`'s
  registry and concrete extractors, and `rusty-hister-crawler`'s `http`
  backend — the rest of Phase 1, tracked in `docs/roadmap/ROADMAP.md` as
  separate follow-up increments.

---

## Confirm rusty_hister's AGPL test-fixture licensing policy
**2026-09-12** · branch [`claude/confirm-hister-licensing`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/confirm-hister-licensing)

Confirms the last open item from `rusty_hister`'s bootstrap (ADR-0001 §7),
on the user's (baileyrd/Nano's) direct instruction. Docs only.

- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0001-…md` §7 →
  confirmed. `rusty_hister` ships under this workspace's standard `MIT OR
  Apache-2.0`; no Hister source file — test files included — is copied
  verbatim into this cluster. Fresh Rust tests are written from
  independently reading and understanding each Go test's behavior instead,
  traceable via the capability inventory's per-extractor test-file
  citations. Binds every future PR touching extractor or query-grammar
  tests, not just this bootstrap.
- **Changed:** `docs/PROJECT-STATUS.md`, `docs/roadmap/ROADMAP.md`, and
  `WORKFLOW.md` updated: Phase 0 is now fully complete (all three
  decision items resolved — crate split/scope, search/crawler approach,
  and now licensing); no items block Phase 1-4 implementation start.
- **Not done here:** no implementation of any kind — this is a licensing
  policy confirmation, not code.

---

## Decide rusty_hister's ADR-0002 and ADR-0003
**2026-09-12** · branch [`claude/decide-hister-adr-0002-0003`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/decide-hister-adr-0002-0003)

Decides the two open decision-requests from `rusty_hister`'s bootstrap
(previous entry below), on the user's (baileyrd/Nano's) direct instruction
("Decide ADR-0002 and ADR-0003 now"), ahead of ADR-0002's own recommended
scoping spike — each ADR records that as an accepted risk, not a silently
skipped step. No implementation code changes; docs only.

- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0002-…md` → Accepted.
  Engine: `rusty_search` + `rusty-search-sqlite-fts5`, chosen over Tantivy
  for single-SQLite-file cohesion with the model DB and with `sqlite-vec`
  (below). Multi-language `IndexAlias` federation, `url_re:` custom
  filtering, and the three highlight styles are decided as
  `rusty-hister-indexer`-layer composition over `rusty-search-core`'s
  `Query` tree, not changes to the shared `rusty_search` crate. BM25 parity
  accepted as result-set, not byte-exact. Semantic-search storage:
  vendored `sqlite-vec` C extension (SQLite path), `pgvector` (Postgres
  path) — no pure-Rust reimplementation for v1.
- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0003-…md` → Accepted.
  CDP crawler backend: `chromiumoxide` as a Tier A adapter dependency (root
  ADR-0002's tiers), since Hister's own `chromedp` backend already wraps an
  external library rather than hand-rolling CDP. WebDriver BiDi: **explicitly
  descoped for v1** (not deferred) — its only advantage over CDP (no
  driver-binary/library dependency) is moot once `chromiumoxide` is already
  accepted; the Notion extractor's JS-rendering requirement is unaffected,
  satisfied by the CDP backend.
- **Changed:** `docs/PROJECT-STATUS.md` and `docs/roadmap/ROADMAP.md`
  updated: Phase 3 (indexer + vectorstore) and Phase 4 (CDP crawler
  backend) are unblocked; Phase 4's BiDi-equivalent line item is marked out
  of v1 scope rather than merely blocked. Crate module doc comments in
  `rusty-hister-{indexer,vectorstore,crawler}` updated to match.
- **Not done here:** no implementation of either decision — the indexer,
  vectorstore storage, or CDP crawler backend. That's Phase 3/4 work,
  tracked but not started.

---

## Bootstrap rusty_hister: a Rust port of asciimoo/hister
**2026-09-12** · branch [`claude/hister-rust-port-quq3ho`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-rust-port-quq3ho)

Bootstraps a Rust port of [asciimoo/hister](https://github.com/asciimoo/hister)
(AGPL-3.0-or-later) as a native crate cluster under `crates/rusty_hister/` —
fresh work written directly in this workspace, not a `git subtree` import of
a pre-existing standalone repo. No port implementation logic lands in this
PR; it establishes the scaffold everything else depends on.

- **Added:** eight new workspace members —
  `rusty-hister-{core,model,extractor,indexer,vectorstore,crawler,server,mcp}`
  — each an empty skeleton crate with a module doc comment pointing back to
  the capability inventory and relevant ADR. All compile clean
  (`cargo check` across the set).
- **Added:** a full `rust-migration`-style capability inventory built from
  reading Hister's actual Go source at commit `49b727f4` (not the kickoff
  brief's own rough orientation notes, which were explicitly unverified) —
  `crates/rusty_hister/docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`.
  Covers 39 HTTP routes, 3 MCP tools (with verbatim prompt-injection-defense
  text), ~35 CLI subcommands, 20 extractors, the full query-language
  grammar, the vectorstore/embedding pipeline, 10 DB models, 3 crawler
  backends, and the TUI, each flagged `[TESTED]`/`[UNTESTED]` against
  Hister's own Go test suite.
- **Added:** a sovereignty audit confirming most of what this port needs is
  already covered by existing first-party crates — `rusty_tokio`,
  `rusty_http`/`rusty_request`, `rusty_tls`, `rusty_json`, `rusty_db`,
  `rusty_url`, `rusty_llama`/`rusty_provider`, and notably `rusty_mcp`
  (already a mature MCP server framework this port's tool surface builds on
  directly instead of a fresh JSON-RPC layer). `rusty_search` covers real
  BM25 today but only a structured query-builder DSL, not a text grammar,
  and no vector search yet. Nothing in the workspace touches the Chrome
  DevTools Protocol or WebDriver BiDi.
- **Added:** three ADRs under `crates/rusty_hister/docs/decisions/` —
  ADR-0001 (accepted: native workspace crates, v1 scope is backend-only per
  the kickoff brief, the crate split and its two revisions from the brief's
  starting suggestion, and an open licensing recommendation for Go test
  fixtures), ADR-0002 and ADR-0003 (both **Proposed**, open
  decision-requests per the kickoff brief's explicit instruction — search/
  indexing engine approach and JS-rendering crawler approach, respectively
  — neither decided in this PR).
- **Not done here:** no indexing, crawling, extraction, or server logic.
  `rusty-hister-indexer`, `rusty-hister-vectorstore`'s storage side, and
  `rusty-hister-crawler`'s JS-rendering backends are explicitly blocked on
  ADR-0002/ADR-0003 sign-off; see `crates/rusty_hister/docs/roadmap/
  ROADMAP.md` for what can proceed in parallel.

## Make rusty_json's serde dependency optional
**2026-09-11** · branch [`claude/clever-wright-y6qkyq`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/clever-wright-y6qkyq)

Resumes `repo-inspector-report.md` Section 1 row 7 (hand-rolled JSON
`Value` in `rusty_oauth`/`rusty_request`), left blocked in an earlier pass
on `rusty_json` having a non-optional `serde` dependency — contradicting
the exact "no `serde`" rationale both hand-rolled crates state in their own
doc comments.

- **Added:** a `serde` Cargo feature on `rusty_json`, on by default (zero
  behavior change for its 16 existing workspace dependents — none needed
  any change, spot-checked). With `default-features = false`, the crate
  pulls in no `serde` dependency at all: `Value` parsing
  (`s.parse::<Value>()` / `Value::from_json_str`) and writing
  (`Value::to_json_string`/`Value::to_json_string_pretty`) go through a new
  direct recursive-descent path (`src/value_io.rs`) that reuses the
  existing hand-rolled tokenizer (`src/parser.rs`) and `Formatter` trait
  instead of `serde::Deserializer`/`Serializer`. String-escaping logic
  moved to a shared `src/escape.rs` so the serde-based and serde-free
  writers can't drift apart. Verified standalone: full test suite green in
  both feature configurations (`cargo test -p rusty_json` and
  `--no-default-features --features std`), plus clippy clean in both.
- **Not done here:** `rusty_oauth` and `rusty_request` still hand-roll
  their own `Value` — this PR only removes the prerequisite blocking their
  migration to `rusty_json`, which touches 14 and 4 call sites
  respectively and is left as a separate, deliberately-scoped follow-up.

---

## Migrate rusty_multimodal_db into the monorepo
**2026-09-10** · branch [`claude/loving-bell-kntf9l`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/loving-bell-kntf9l)

`baileyrd/rusty_multimodal_db` — a benchmark harness comparing AoS, SoA,
and UUID-canonical-store record backends, plus the production store,
network server, and schema-driven client built on the winning design —
merged into `crates/rusty_multimodal_db/` via `git subtree`, full history
preserved. A sixth merge outside the `baileyrd/rusty_*` wave numbering
(ADR-0001), same treatment as the `nexus` merge above.

- **Added:** `crates/rusty_multimodal_db` joins this root's member list
  directly (it was already a single, non-nested `Cargo.toml`, unlike
  `nexus`/`rusty_agent_gateway`/`rusty_yirp` — nothing to de-nest).
- **Changed:** its one pinned git dependency on `rusty_tls`
  (`Rusty-Mill/rusty_mill` at a specific commit) retired to a plain path
  dependency on this workspace's own `crates/rusty_tls` — the
  same-workspace-source rule ADR-0002 requires, and the same swap
  `rusty_yirp`'s `sessionmgr-pty` and `nexus-rush` made on their own
  merges.
- **Changed:** its `rusqlite` pin (used only by the optional
  `external-db-bench` benchmark feature) bumped `0.32` → `0.39` to match
  `crates/rusty_inventrory`'s `inventory-core` — `rusqlite` declares
  `links = "sqlite3"`, and Cargo allows only one version of a
  `links`-declaring crate in the whole dependency graph; unifying on the
  higher version is the same fix the `nexus` merge's `sqlx`/`rusqlite`
  collision needed, not a behavior choice of this crate's own.
- **Fixed:** two `clippy::chunks_exact_to_as_chunks` failures
  (`src/durability/mmap_store.rs`, `src/server/pem.rs`) — this workspace's
  clippy version flags `chunks_exact(N)` with a constant `N` in favor of
  `as_chunks::<N>().0`; behavior unchanged, same trailing-partial-chunk
  drop either way. The upstream repo hit the identical failure on its own
  `main` (unrelated to this merge — its clippy toolchain updated
  independently) and carries the same fix.
- **Changed:** `rusty_multimodal_db` added to the `windows-latest`
  `windows-exclude` list alongside `rusty_stream`/`rusty_fedora_agent` —
  its optional `external-db-bench` feature's `duckdb` dependency vendors
  DuckDB's own C++ amalgamation, and this workspace's `--all-features` is
  what first compiles it on `windows-latest`; that native build fails
  under the runner's current MSVC toolchain (a third-party build issue,
  no Rust-side fix available here). The upstream repo's own CI never ran
  a Windows job at all, so this wasn't a regression, just first exposure.
- **Verified:** `cargo tree -p rusty_multimodal_db --all-features`
  resolves to one `rusqlite v0.39.0`; `cargo check -p rusty_multimodal_db
  --features research,server,perf-events` compiles clean, and separately
  `--features research,external-db-bench` compiles clean too (DuckDB's
  bundled from-source build, already a documented one-time cost — see
  this crate's own `docs/decisions/ADR-0015-external-database-benchmark.md`
  — checked on its own given how long that build takes, ~10.5 minutes).

## Migrate nexus into the monorepo
**2026-09-08** · branch [`claude/nexus-rusty-mill-migration-ic3fqa`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/nexus-rusty-mill-migration-ic3fqa)

`baileyrd/nexus` — a 42-crate microkernel note-taking/AI-agent workspace —
merged into `crates/nexus/` via `git subtree`, full history preserved. A
fifth merge outside the `baileyrd/rusty_*` wave numbering (ADR-0001).

- **Added:** all 42 of nexus's workspace crates join this root's member
  list directly; `crates/nexus/shell` (its Tauri desktop shell) is
  `exclude`d the same way as `rusty_key`'s `desktop/src-tauri`.
- **Changed:** nexus's own nested `[workspace]`, `Cargo.lock`, and
  `.cargo/config.toml` (a `target-dir` override that would have
  fragmented this workspace's shared `target/` for anyone building from
  inside `crates/nexus/`) were dropped on merge, the same treatment
  rusty_agent_gateway's and rusty_yirp's own nested workspaces got.
  nexus's `rustils` git pins (`platform`/`platform-linux`) retired to
  this root's existing path dependencies (ADR-0002) — the same swap
  rusty_yirp's `sessionmgr-pty` made.
- **Changed:** `sqlx` bumped `0.8` → `0.9` workspace-wide to resolve a
  hard `libsqlite3-sys` version collision between `sqlx-sqlite` (used by
  `rusty_db`'s SQLite driver and `rusty_acp`'s optional Postgres store)
  and the `rusqlite 0.39` nexus's storage layer needs — both crates
  register `links = "sqlite3"`/pull `libsqlite3-sys`, and Cargo allows
  only one version of a `links`-declaring crate in the whole graph.
  `rusqlite` itself bumped `0.32.1` → `0.39` at the root for the same
  reason; `inventory-core`, `rusty_sqlite`, and `rk-feed` (each
  previously pinned lower for their own documented reasons, all still
  satisfied at `0.39`) now share that one version too. sqlx 0.9's new
  `SqlSafeStr` injection-audit bound needed `sqlx::AssertSqlSafe` wraps
  at every dynamic-SQL call site in `rusty-db-postgres`/`-mysql`/
  `-sqlite` and `rusty_acp`'s postgres store — all of them build SQL
  from a config-supplied table prefix or a caller-supplied connection
  hook, never request data, so each wrap is an audit assertion, not a
  behavior change. This was surfaced to, and approved by, the user
  before making the bump (a toolchain/dependency change, per the
  working agreement) rather than picked unilaterally.
- **Fixed:** two latent nexus-side gaps that its own CI never exercised,
  only surfaced once this workspace's `--all-features` build compiled
  them: `nexus-memory`'s `Memory`/`MemoryType`/`MemoryStatus` never
  derived `TS`/`JsonSchema` despite being embedded in a `ts-export` IPC
  type (`nexus-memory` isn't in nexus's own `check_ipc_drift.sh` build
  list); `nexus-rush`'s job-spawn `Command` literal was missing a
  `detached` field added to this workspace's `rustils` fork after
  nexus's former pinned rev.
- **Verified:** `cargo check --workspace --all-features` across all 230
  workspace members (42 of them new from nexus), `cargo fmt --all -- --check`,
  and `.github/scripts/check_workspace_deps.py` (ADR-0002 dependency-policy
  check) all pass clean.

## Work through the repo-inspector report's duplication and sovereignty rows
**2026-09-05** · branch [`claude/repo-inspector-report-wgoocq`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/repo-inspector-report-wgoocq)

Every row of `repo-inspector-report.md` (regenerated in #152) now has a
recorded disposition in a new **Disposition** section at the top of the
report; the actionable ones landed here.

- **Added:** `crates/rusty_rand` — one dependency-free OS CSPRNG
  (`/dev/urandom` cached handle / `BCryptGenRandom`) replacing three
  identical copies in `rusty_oauth`, `rusty_uuid`, and `sessionmgr-proc`
  (the third was not in the report; it was indexed under `os_random`).
  Public APIs of all three consumers are unchanged.
- **Added:** `rusty_simd::f32_to_f16`; `rusty_llama` and `rusty_whisper`
  re-export both f16 directions from `rusty_simd` and delete their own.
  The two deleted `f32_to_f16` copies disagreed (half-up rounding vs.
  ties-to-even; one mapped NaN to infinity) — both were test-fixture-only,
  so no shipped code path changes.
- **Added:** `rusty_wiremock::canned` behind a `std` feature — the
  sequential canned-response mock server that `rusty_proxmox`,
  `rusty_opnsense`, `rusty_fedora`, and `rusty_homelab_mcp` each carried
  an identical copy of; all four now dev-depend on it. `rusty_wiremock`
  was the home each copy's own doc comment named while calling it a stub.
- **Changed:** `rusty_base64` decodes strictly (misplaced/excess padding
  and non-4-aligned padded input are errors; `DecodeError` carries
  positions) so `sessionmgr-protocol` could adopt it without giving up its
  "reject, never guess" rule. With that, the last three hand-rolled base64
  copies and the last four external `base64` users are gone: no workspace
  manifest declares external `base64` any more.
- **Changed:** `adk-core` → `rusty_uuid`; `rk-feed` → `rusty_url`
  (direct edge only — `reqwest` keeps external `url` transitively).
- **Not done, with reasons recorded in the report:** row 7 (JSON `Value`)
  is blocked because `rusty_json` has a non-optional `serde` dependency,
  which is the one thing `rusty_oauth`/`rusty_request` hand-roll to avoid
  — a serde-free `rusty_json` surface is the prerequisite. `rusty-acp`/
  `rusty-db-core` keep external `uuid` (serde + `sqlx` type mapping on the
  `Uuid` type); `sessionmgr-proc` keeps `libc` (same Track P
  dual-backend decision as #120); `rustls` in `agentgateway-tls`/
  `rp-router` and `rusqlite` in four crates were checked for fit against
  `rusty_tls`/`rusty_sqlite` and there is none today; `toml` needs a serde
  `Deserializer` and `[[array-of-tables]]` in `rusty_codec` first. Row 2
  (retry) turned out narrower than reported: `rusty_request` and
  `rusty_acp` already delegate backoff to `rusty_retry`.
- **Verified:** `cargo test` on every touched crate plus every
  `rusty_base64::DecodeError` consumer (`rusty_a2a`, `rusty-acp`,
  `rusty-mcp`) and `sessionmgr-daemon` (the one test that drives a real
  `claude` binary was skipped as environment-dependent); `cargo fmt --all
  -- --check`; `cargo clippy --all-targets --all-features -- -D warnings`
  on all touched crates; `.github/scripts/check_workspace_deps.py` clean.

---

## Add a Fedora/systemd module to rusty_homelab_mcp
**2026-09-04** · branch [`claude/fedora-systemd-module-hb7vxv`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/fedora-systemd-module-hb7vxv)

- **Added:** `crates/rusty_fedora_agent` — an unprivileged local agent for
  a Fedora Server host with no REST management API of its own (e.g.
  baileyai). Exposes `fedora_system_status`/`fedora_list_services`/
  `fedora_service_control`/`fedora_read_journal`/`fedora_dnf_list_updates`/
  `fedora_dnf_install`/`fedora_dnf_remove`/`fedora_task_status`/
  `fedora_read_config`/`fedora_write_config`'s ten operations over a small
  synchronous HTTP API (`tiny_http`), built on `rustils`'
  `platform`/`platform-linux` `Spawner`/`Command` for `systemctl`/
  `journalctl`/`dnf`, with `SystemController`/`PackageController` domain
  ports so tool-handling logic can be tested against `platform-mock`'s
  scripted spawner without a real Fedora box.
- **Added:** `crates/rusty_fedora` — async typed client for that agent's
  HTTP API, matching `rusty_opnsense`/`rusty_proxmox`'s shape exactly
  (built on `rusty_request`, passthrough JSON, no MCP dependency).
- **Added:** `rusty_homelab_mcp` gained a `fedora` module: the ten tools
  above, following the existing OPNsense/Proxmox discovery-then-mutate
  pattern and `$defs` oneOf-enum style (`ServiceActionArg`, `UnitTypeArg`,
  `PriorityArg`) exactly. `HomelabServer::new` now takes a third,
  independent, optional `FedoraAgentClient`.
- **Not a new repo:** the handoff brief for this task assumed
  `rusty_homelab_mcp`/`rustils` were still separate GitHub repos (as they
  were before this monorepo's consolidation) and proposed a new
  standalone `rusty_fedora_agent` repo. Verified against the actual repo
  before writing anything and built it as a workspace crate instead —
  `ARCHITECTURE.md` documents this monorepo deliberately consolidating
  ~90+ formerly-independent repos for one CI/one place to de-duplicate,
  and a new standalone repo would fight that policy on day one.
- **Privilege scoping is deliberately not applied automatically:**
  `crates/rusty_fedora_agent/deploy/` ships a systemd unit, a polkit rule
  (unit allowlist), a sudoers `NOPASSWD` entry (`dnf install`/`remove`
  only, package-name scoping enforced inside the agent before `dnf` is
  ever invoked), and an allowlist config — all reviewable templates a
  human applies to the target host by hand, with an **empty** allowlist
  by default (nothing permitted until deliberately added). This session
  has no access to the real target host (baileyai), so the "real
  happy-path run against baileyai" a full rollout calls for is left to
  whoever applies `deploy/`.
- **Verified:** `cargo check --workspace` and `cargo test -p
  rusty_fedora_agent -p rusty_fedora -p rusty_homelab_mcp` (allowlist
  rejection tests, scripted-spawner happy-path tests, and mock-HTTP MCP
  tool-dispatch tests).

---

## Fix `rusty_tokio`'s Windows reactor orphaning a socket on a failed AFD re-arm
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `crates/rusty_tokio/src/io/reactor/windows.rs`'s `event_loop`
  discarded the result of resubmitting a socket's one-shot
  `IOCTL_AFD_POLL` after every completion (`let _ =
  self.submit_poll(&state)`, at both call sites). If that resubmission
  itself failed, no further completion would ever arrive for that
  socket, so a `readable()`/`writable()` wait registered on it
  afterward hung forever with nothing left to wake it — observed as
  four unrelated `rusty_tokio`/`rusty_tls` tests intermittently timing
  out at nextest's ~600s slow-timeout on `test (windows-latest)`, then
  passing in milliseconds on the very next retry ([#153](https://github.com/Rusty-Mill/rusty_mill/issues/153)).
- **Why not the earlier readiness-edge fix ([#140](https://github.com/Rusty-Mill/rusty_mill/pull/140)):** all four
  occurrences happened on runs *after* #140 had already merged, and its
  fix targets a different failure shape (a bit cleared out from under a
  fresh edge) than this one (no bit update ever happens again because
  nothing is watching the socket anymore).
- **How:** both re-arm call sites now check `submit_poll`'s result and,
  on failure, mark both directions ready via a new `mark_orphaned`
  helper — the same "surface both directions so the caller's own next
  syscall discovers the truth" pattern `event_loop`'s sibling
  bad-completion-status branch already used for a different failure
  mode, extended to cover this one.
- **Verified:** `cargo check`/`clippy -D warnings` clean on
  `x86_64-pc-windows-gnu` (cross-compiled from this Linux sandbox, which
  cannot execute Windows tests); the full Linux `rusty_tokio` suite
  passes unaffected (the changed code is `#[cfg(windows)]`-only). Real
  verification comes from `windows-latest` CI itself, the same oracle
  #137/#138/#140 relied on.
- **Known limitation — partial fix:** the `windows-latest` CI run on
  this very branch (33784080891) reproduced the identical hang
  signature on `rusty_tls::async_handshake::async_handshake_succeeds_and_round_trips_with_pinned_anchor`
  (TRY 1 timing out at ~600s, TRY 2 passing instantly) with this fix
  already applied. So this change is real and worth keeping — it closes
  a genuine silently-swallowed-error hole — but it does not fully
  resolve #153. #153 stays open, narrowed to the still-unexplained
  remainder.

---

## Extract `rusty_base64`; close issue #119
**2026-09-03** · branch [`docs/rusty-base64-extraction-issue-119`](https://github.com/Rusty-Mill/rusty_mill/tree/docs/rusty-base64-extraction-issue-119)

- **Added:** `rusty_base64` — `rusty_oauth::encoding::base64`'s complete
  surface (encode/decode, standard and URL-safe alphabets) extracted into
  its own crate, per [issue #119](https://github.com/Rusty-Mill/rusty_mill/issues/119).
  `rusty_request`'s own `base64.rs` was ruled out as a base: it's private,
  encode-only, and standard-alphabet-only, and extending it would have
  meant building a second base64 crate when `rusty_oauth`'s already
  covered the need.
- **Changed:** `rusty_oauth` now depends on `rusty_base64` too
  (dogfooding) instead of keeping its own copy — its public
  `encoding::base64::*` path is unchanged, so none of its own call sites
  needed edits. `rusty_acp`, `rusty-mcp`, and `rusty_a2a` swapped their
  external `base64` crate dependency for `rusty_base64` after checking
  each call site's exact API needs (standard vs. URL-safe, padded vs.
  unpadded, encode vs. decode) rather than assuming a blind swap would fit
  — the same per-crate verification issue #119 itself asked for.
- **Fixed:** the extraction rewrites `encode_with`/`decode_with`'s
  chunking from `slice::as_chunks` (`rusty_oauth`'s original) to
  `chunks_exact`/`remainder`. `as_chunks` is not yet stable at
  `rusty_acp`'s own `rust-version = "1.86"` floor — confirmed against a
  real `+1.86` toolchain before merging, since `rusty_acp`'s own CI
  convention runs `cargo +1.86 test` and this would have silently broken
  it. Behaviorally identical (verified via the original RFC 4648 test
  vectors under both a `+1.86` and the workspace's default toolchain).
- Known limitation: `rusty_croc`, `adk-a2a`, `agentgateway-auth`, and
  `agentgateway` still depend on the external `base64` crate. They weren't
  in issue #119's verified evidence (filed before three of them joined the
  workspace) and weren't checked here — left for separate follow-up rather
  than swapped without per-call-site verification.

---

## Fix `sessionmgr-pty`'s intermittent size-reporting flake
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `LinuxPty::spawn` (`crates/rustils/crates/platform-linux`) set
  a session's pty window size *after* spawning the hosted child, so the
  child could run (and, in `sessionmgr-pty`'s size-reporting test, read
  its own terminal size via `stty size`) before the parent's
  `TIOCSWINSZ` ioctl took effect — a race that had been intermittently
  failing `sessionmgr-pty::tests::the_terminal_reports_the_size_it_was_given`
  on `main`'s `test (ubuntu-latest)` CI job (silently absorbed by
  nextest's retry budget on most runs, then hard-failing it outright on
  [PR #149](https://github.com/Rusty-Mill/rusty_mill/pull/149)), tracked
  as [#150](https://github.com/Rusty-Mill/rusty_mill/issues/150). Fixed
  by setting the size on the pty master before the child is spawned,
  closing the race. `platform`/`platform-linux`/`platform-windows`/
  `platform-mock`/`platform-bsd`/`platform-parity` bumped `0.27.0` →
  `0.27.1` (patch-level: no public API shape changed).

---

## Dependency sovereignty policy (ADR-0002) and the last workspace-member git pins
**2026-09-03** · branch [`claude/review-recommended-changes-8kepe6`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/review-recommended-changes-8kepe6)

- **Added:** `docs/adr/0002-dependency-sovereignty-policy.md` — a three-tier
  classification (Sovereign / Transitional / Adapter) for how a crate's
  external dependencies relate to the workspace's dependency-minimizing
  purpose, written in response to an external Atlas-alignment review that
  found "no external dependencies" does not describe the monorepo as a
  whole (114 of 199 manifests declare a direct external normal dependency).
  A generated, per-crate ledger cross-referencing manifests to tiers is
  tracked as follow-up, not introduced here.
- **Fixed:** `crates/rusty_term/l13`, `crates/rusty_font`, and
  `crates/rusty_gpu` depended on `rusty_lsp`/`rusty_simd` via a pinned git
  URL even though both are workspace members with their own `crates/<name>`
  directory, letting the git and workspace copies silently diverge —
  contrary to `ATLAS-RWC-0050`. All three now use plain path dependencies.
- **Added:** `.github/scripts/check_workspace_deps.py` (with unit tests) and
  a new `dependency-policy` CI job that fails a PR if any workspace member's
  name resolves from a git source anywhere in the dependency graph —
  confirmed to catch the exact violation above by running it against the
  pre-fix manifests.
- Known limitation: `main` is still unprotected on GitHub (no required
  status checks, no branch protection rule), so this new CI job — like the
  rest of `ci.yml` — is not yet a merge gate. That requires a repository
  admin action outside what this PR's tooling can perform; see the Atlas
  review's `ATLAS-TOOL-0010`/`0011` findings.

## Review policy: author self-review when no independent reviewer is available
**2026-09-03** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `CONTRIBUTING.md` said "at least one approval required" while
  every PR merged on 2026-09-02 was authored, self-merged, and unreviewed
  by the same account, which the Atlas evidence review (`docs/atlas/`)
  recorded as an unenforced policy. The policy now matches practice
  honestly: an independent approval when a reviewer is reasonably
  available, otherwise a recorded author self-review against the reviewer
  checklist after CI is green. The PR description must say no independent
  reviewer was available; self-review is never represented as independent
  review (the distinction Atlas `ATLAS-GOV-REVIEW-0061`/`0064` draws).
  Security-sensitive, irreversible, or ecosystem-breaking changes still
  wait for an independent reviewer when one can be found.
- **Changed:** the four PR templates gain a checklist line — "Reviewed:
  independent approval, or self-review recorded in the description" — so
  the record is made on every PR rather than remembered.
- Known limitation: this is documented policy, not enforcement. `main`
  is still unprotected, so nothing stops a merge that skips the record.

## Retire the last `rustils` git pins to path dependencies
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `rusty_tokio`, `rusty_tls`, and `rustils_async` (root manifest
  plus `platform-async`, `platform-async-mock`, `platform-async-linux`,
  `coreutils-async`) depended on `platform`/`platform-linux`/
  `platform-bsd`/`platform-windows`/`platform-mock` through rev-pinned
  `git` dependencies on `baileyrd/rustils`, left over from before
  `rustils` joined this workspace. All twenty-one declarations are now
  `path` dependencies on `crates/rustils/crates/<name>`, the same
  retirement every other first-party pin got when its crate merged.
- **Changed:** `Cargo.lock` drops thirteen git-sourced `platform*` entries
  (three checkouts: two at 0.27.0, `rusty_tls`'s at 0.22.1). Each crate now
  resolves to one in-tree 0.27.0 instance, so consumers share one
  `platform::error::PlatformError` type instead of one per checkout.
- **Verified:** `cargo check`, `cargo clippy -D warnings`, and `cargo test`
  with `--all-features --all-targets` across the six consumers on Linux;
  the Windows and BSD backends are compiled only on their targets, so the
  Windows leg of CI is the evidence for `platform-windows` and nothing
  here exercises `platform-bsd`.
- Known limitation: `rusty_tls` moves from platform 0.22.1 to 0.27.0 in one
  step. It compiles and its tests pass, which is the check the versioning
  rule asks for, but any behavioural change in those five minor versions
  reaches `rusty_tls` with this merge.

## rusty_meshed: reverse-trace & domain-maturity crate
**2026-09-02** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Added:** `crates/rusty_meshed/crates/rusty-meshed-trace` -- the core of
  the *Reverse-Trace & Domain Maturity* spec (Phase 1): the five-level
  `Maturity` ladder, `Domain`/`Source`/`Outcome`/`Requirement` types, a pure
  `trace()` that classifies every requirement (satisfied / blocked / degraded
  / missing), caps the outcome's fidelity at its weakest required domain and
  returns a worst-first bottleneck list, TOML scenario loading via
  `rusty_codec`'s sovereign parser, JSON round-tripping via `rusty_json`, a
  Markdown "gap summary" export, and one shipped scenario (*Acquisition
  Status Dashboard*, ten domains, four outcomes). Fifteen fixture tests cover
  every verdict and edge class, ordering, what-if, and both file formats.
- **Changed:** the crate is a new workspace member; `rusty_meshed/README.md`
  gains a crate-table row and a section on the new capability.
- Known limitation: the shipped scenario's maturity levels are illustrative
  placeholders (spec open question #3), not an assessment; the renderer
  (Phase 2) lives in the source repo's `data-mesh-monitor`, not here.

## Chore: drop committed Python bytecode, ignore it going forward
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** the previous PR ran the new `.github/scripts` unit tests
  locally before staging and swept two `__pycache__/*.pyc` files into the
  commit. Removed from the tree; `__pycache__/` and `*.pyc` added to
  `.gitignore` so it cannot recur.

## CI: unit tests for the affected-crates plan step
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `.github/scripts/test_affected_crates.py` plus a `plan-tests`
  CI job. The plan step decides what every other job runs on a PR, and the
  Atlas review flagged that its nested-crate ownership and
  reverse-dependency traversal had no regression tests. Thirteen cases
  cover: a file inside a crate, outside every crate, the manifest itself,
  a nested crate winning over its parent, `crates/foo` not claiming
  `crates/foobar`, direct and transitive dependents, leaf changes not
  pulling in dependencies, non-workspace dependencies ignored, cyclic
  dev-dependency graphs terminating, sorted/deduplicated output, and a
  member missing from the resolve graph.
- **Changed:** `affected_crates.py`'s graph logic moved into
  `affected_packages(metadata, changed_files)` with type hints; the CLI
  contract (metadata path in argv, changed files on stdin, names on
  stdout) is unchanged and was checked against the real workspace metadata
  for four representative inputs.
- Known limitation: the tests use synthetic metadata; the end-to-end check
  that CI actually scopes to the right crates remains PR #68's round-trip
  test.

## Docs: repository-map corrections from the Atlas review
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** README relationship text that the Atlas review found stale:
  `rusty_simd` was described as the one crate "still outstanding" while
  merged and listed as a member; `rusty_tokio` was described as having no
  in-repo dependents while nineteen workspace packages depend on it by
  `path`; `rustils` was described as outside the monorepo's scope while
  living at `crates/rustils`. The surviving `git` pins on `rustils` in
  `rusty_tokio`, `rustils_async`, and `rusty_tls` are now stated as
  outstanding rather than implied to be by design.
- **Fixed:** `ARCHITECTURE.md` described ATLAS-300 as a seed too draft to
  cite; it is an active volume since Atlas ADR-0006. The section now points
  at `docs/atlas/` for the requirement-by-requirement crosswalk.
- **Fixed:** the Atlas review's `rusty_tokio` dependent count, which was
  taken from a manifest grep that matched a comment in `rusty_proxmox`;
  now taken from `cargo metadata --all-features` at the evidence revision.
- Known limitation: docs only. The `rustils` pin retirement itself is not
  done here.

## Atlas evidence review — revision 2 with corrections
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review.md` — the review
  of this monorepo as exercised evidence for the Atlas Engineering
  Standards Library, revised after every claim was verified against
  Rusty Mill `06ca8669`, the live PR/branch state, and Atlas `390d6b0f`.
  Concludes that ATLAS-300's deferred feature-flag trigger fired (PRs #134
  and #136), and lists the governance corrections this repo needs before
  any conformance claim: protect `main`, enforce the documented review
  policy, and fix the stale README/ARCHITECTURE map.
- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review-corrections.md`
  — the must-fix and should-fix items found in the review's first
  revision, each with the evidence that established it.
- Known limitation: the review is an alignment assessment, not a
  certification, and PR #131 (still open) is excluded from its evidence
  revision.

## Fourth-wave merge — `rusty_agent_gateway` (wave complete)
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_agent_gateway` — a Rust implementation of the
  agentgateway data plane, built as a drop-in for its `config.yaml`:
  configuration, listeners, route matching, policies, and the MCP gateway
  (several upstream MCP servers federated behind one endpoint, with
  tool-level filtering and authorization). Nine crates behind one nested
  workspace, merged via `git subtree` with full history. This completes the
  fourth wave and the monorepo consolidation.
- **Changed:** four pins retired, the most of any crate in this series, all
  to merged siblings — `rusty_a2a` (rev `b9778e1`, 11,324/360 behind),
  `rusty-mcp` (tag `v0.4.1`, 1,367/210 behind — the only tag-pinned
  dependency in the series), `rusty_tls` (rev `7ac6956e`, 109/27) and
  `rusty_tokio` (rev `6d3bb05a`, 3,158/587). The last two had to move
  together by construction, the same `AsyncRead`/`AsyncWrite` trait-identity
  constraint `rusty_request`'s retirement documented.
- **Changed:** this root's `rusty_a2a` and `rusty-mcp` entries now carry
  `default-features = false`, because a member inheriting a workspace
  dependency may not set it when the root does not — and the gateway's
  crates set it deliberately. Verified to be a no-op for their other
  consumers (`adk-a2a`, `rp-mcp`, `rp-server`): both crates' `default`
  feature is empty.
- **Fixed/Changed:** `[workspace.package]` collided on five fields, so its
  crates carry literal `[package]` fields; its `[workspace.lints]` is
  stricter than this root's (`unsafe_code = "forbid"`, `missing_docs`,
  `clippy::todo`, `clippy::unwrap_used`) and is written literally into each
  crate rather than silently downgraded — same call as `rusty_key`'s. Root
  `clap` gained `env`.
- **Known limitation (pre-existing, unchanged):** `hyper`'s `http2` feature
  is load-bearing for the shipped `agentgateway` binary (its TLS listener
  advertises `h2` over ALPN) but Cargo's feature unification means the test
  binary has it regardless — so it must be verified against a built binary
  with `curl`, not by `cargo test`, exactly as before the merge.
- 73 tests pass. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_yirp`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_yirp` — sessionmgr, a Windows-native session
  manager for AI coding-agent CLIs (Claude Code, Codex, Gemini CLI): each
  session optionally in its own git worktree, a TUI grid dashboard, and
  sessions that survive the manager closing. Eight `sessionmgr-*` crates
  plus a Tauri 2 desktop shell behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** four pins retired — `rusty_tokio` (rev `6e6f1847`, from its
  own `[workspace.dependencies]`) and `sessionmgr-pty`'s `platform`,
  `platform-linux`, `platform-windows` (`rustils` rev `ce9259d4`) — all now
  this root's path entries. `sessionmgr-pty`'s manifest warned that a
  second, differing `rustils` pin would build two non-interoperating copies
  of the platform layer; this wave merged a third consumer
  (`rusty_tailscale`, at a different rev again), and a `path` dependency
  settles that by construction.
- **Changed:** `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields.
- **Known limitation:** `sessionmgr-daemon`'s
  `a_fresh_claude_session_reaches_needs_input_on_its_own` drives a real
  `claude` session and skips when `claude` is not on `PATH` — the state of
  a CI runner. On a machine with the CLI installed but no way to complete
  its interactive trust prompt, the guard passes and the test times out.
  Same class as `mill-term`'s known environment-dependent failure. With
  `claude` off `PATH` the suite is 130/130.
- All eight non-Tauri crates cross-compile for `x86_64-pc-windows-gnu`. No
  lint or format fixes were needed.

## Fourth-wave merge — `rusty_provider`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_provider` — an AI provider router: one
  OpenAI-compatible HTTP API in front of OpenAI, Anthropic, Gemini, Groq,
  Together AI and Fireworks, with config-driven fallback chains, budgets,
  metrics, an MCP surface and a CLI. Six crates behind one nested
  workspace, merged via `git subtree` with full history.
- **Changed:** its branch-tracking `rusty-mcp` git dependency retired to a
  `path` dependency on the merged sibling. It had resolved to `ee6c7637` —
  six commits behind the commit this workspace imported, plus two since.
  Verified by running the group's full suite (905 tests) against the swap.
- **Changed:** `[workspace.package]` collided on `license`, so its crates
  carry literal `[package]` fields. Root `reqwest` gained `stream` (SSE
  deltas from upstream providers) and root `tokio` gained `full`, declared
  at the root because Cargo unifies features across the graph either way.
- 905 tests pass. No lint or format fixes were needed — `rusty_provider`
  is the first crate group in this wave to arrive already clean under this
  workspace's `-D warnings` gate and `cargo fmt`.

## Fourth-wave merge — `rusty_adk`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_adk` — a Rust port of the Agent Development Kit
  (ADK) 2.0 architecture: the same data model, graph execution engine, and
  tool/callback contracts, plus MCP and A2A bridges. Eleven library crates
  and three runnable examples behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** `adk-a2a`'s `rusty_a2a` dependency retired from a
  *branch-tracking* git dependency (no `rev`, unlike every other pin in
  this series) to a `path` dependency on the merged sibling. What it had
  actually resolved to was 42 commits behind the commit this workspace
  imported — 9,729 insertions across 66 files — plus three commits since,
  one of which changed `require_auth`'s error type. Verified by running
  `adk-a2a`'s own suite against the swap (13 unit, 8 end-to-end, 10
  remote-transport tests), not by reading the diff.
- **Changed:** `[workspace.package]` collided on `rust-version`, `license`
  and `repository`, so its crates carry literal `[package]` fields. Root
  `tokio` gained `io-std`, `uuid` gained `serde`, and `serde_json` gained
  `float_roundtrip` (which `rusty_adk`'s SQLite session store needs for
  exact f64 round-trips, and which Cargo unifies globally anyway, so it is
  declared where it is visible). `thiserror` and `schemars` stay literal on
  the `adk-*` crates — `"2"` and `"0.8"` against this root's `"1"` and
  `rusty_key`'s `"1.0.4"`.
- **Fixed:** `adk-sessions`' optional `rusqlite = "0.37"` moved to this
  root's `"0.32.1"` — the same `libsqlite3-sys` `links` conflict
  `inventory-core` hit, since Cargo's uniqueness check counts optional
  dependencies it never activates.
- 278 tests pass, 2 ignored. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_tailscale`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_tailscale` — a sovereign pure-Rust Tailscale
  client: ts2021 control plane, WireGuard data plane, DERP/STUN/disco NAT
  traversal, a userspace smoltcp stack, a daemon and a CLI. Fifteen `ts-*`
  crates plus `xtask` behind one nested workspace (whose `members` was a
  `crates/*` glob, expanded to literal entries here), merged via
  `git subtree` with full history.
- **Changed:** `[workspace.package]` collided on `version`, `edition`
  (2024 — the first edition-2024 crates here) and `repository`, so its
  crates carry literal `[package]` fields; only its dependencies were
  hoisted, with `rusty_http`/`rusty_crypto_key` re-pathed to this root's
  `crates/` layout.
- **Changed:** `ts-magicsock` and `ts-tun`'s pinned `rustils` git
  dependencies (`platform`, `platform-linux`, rev `b8bf992f`) retired to
  this root's path entries, same as `rusty_rdp`'s.
- **Fixed:** two pre-existing breaks in `rusty_tailscale`'s own `main` —
  it has no CI of its own and does not compile on Linux. `ts-magicsock`
  called three `platform::net::UdpSocket` trait methods without the trait
  in scope (true at the pinned rev too, so not drift the pin was hiding),
  and `ts-cli`'s `localapi::Error` declared a `Status(StatusCode)` variant
  nothing constructs while `request()` constructed a nonexistent
  `Api { status, body }`. Both confirmed against the standalone repo first.
- **Fixed:** four more `generic-array` 0.14.9 deprecations (`ts-control`'s
  Noise handshake and frame codec, `ts-disco`, `ts-derp`), plus a
  `cargo fmt --all` pass.
- 93 tests pass. All sixteen crates also cross-compile for
  `x86_64-pc-windows-gnu` despite the Linux-first design, so — unlike
  `rusty_stream` — no `windows-exclude` was needed.

## Fourth-wave merge — `rusty_llama`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_llama` — a from-scratch Llama/GGUF inference
  engine: CPU SIMD kernels, optional `wgpu` and CUDA backends, an
  OpenAI-compatible server, and GGUF-embedded Jinja chat templating. Merged
  via `git subtree` with full history.
- No dependency swaps: its `rusty_simd`/`rusty_std` path dependencies
  already pointed at siblings under `crates/`.
- **Fixed:** two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures. They only appear with the `cuda` feature on, which this
  workspace's `--all-features` clippy gate does and the crate's own CI
  never did.
- **Fixed:** `render_jinja_threads_context_variables` asserted a bool
  interpolates as `true`. `minijinja` 2.22 deliberately changed
  none/bool rendering to `None`/`True`/`False` for Jinja2 compatibility;
  the standalone lockfile pinned 2.21, this workspace resolves 2.24. Since
  the code path exists to render templates authored for Python Jinja2, the
  new rendering is the correct one — assertion updated, reason recorded
  inline. Nothing else in the crate interpolates a bare boolean.
- **Changed:** reformatted with `cargo fmt --all` (not fmt-clean under this
  workspace's settings).
- 248 tests pass; 49 stay ignored because they need real model weights on
  disk, by the crate's own design.

## Fourth-wave merge — `rusty_key`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_key` — Rusty Keys, an AI-native application
  skeleton where the model's agent loop is the kernel and the application
  is the harness around it (constrain / feed / observe / compose). Eight
  crates behind one nested workspace, merged via `git subtree` with full
  history.
- **Changed:** its `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields and only its
  dependencies were hoisted (`aisdk`, `schemars`, `toml`, and the `rk-*`
  path entries).
- **Fixed:** `rusty_key`'s `[workspace.lints]` (`unsafe_code = "forbid"`)
  is strictly stronger than this root's, which is `rustils`'
  (`unsafe_code = "warn"`). Leaving `[lints] workspace = true` in place
  would have silently downgraded all eight crates, so each carries a
  literal `[lints.rust] unsafe_code = "forbid"`; `rustils`' crates keep
  inheriting the root table unchanged.
- **Changed:** `crates/rusty_key` reformatted with `cargo fmt --all` — it
  was not fmt-clean under this workspace's settings, same as
  `rusty_ansder`/`rusty_boot` when they merged. No behavior change.
- **Known limitation:** its Tauri desktop shell
  (`crates/rusty_key/desktop/src-tauri`) stays a standalone workspace and
  is excluded here, exactly as its own repo had it — the opposite call from
  `inventory-tauri`, and deliberately so, since each is upstream's own.
  Verified it still builds across the boundary post-merge.
- **Known limitation:** two more duplicate-major pairs now resolve —
  `rmcp` 0.9.1 alongside 3.1.4, and `axum` 0.7.9 alongside 0.8.9. Cargo
  keeps them as unrelated crates and no type crosses between the groups.
- No dependency swaps. Full suite (194 tests) passes unmodified.

## Fourth-wave merge — `rusty_skillopt`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_skillopt` — a from-scratch Rust take on
  Microsoft's SkillOpt: optimize a skill markdown document as the trainable
  state of a frozen LLM agent, with epochs, batches and a validation gate,
  entirely in text space. Four crates behind one nested workspace, merged
  via `git subtree` with full history.
- **Changed:** its `[workspace.package]` collided on `license`, so its four
  crates carry literal `[package]` fields (the `rusty_db` treatment) and
  only its dependencies were hoisted. Root `tokio` widened to the union of
  what `rusty_search`, `rusty_db` and `rusty_skillopt` need; root `chrono`
  gained `serde`. `thiserror` stays literal on `skillopt-core`/
  `skillopt-model`, same `"2"`-vs-`"1"` reason as before.
- **Known limitation:** the workspace now resolves two `reqwest` majors —
  0.13.4 for `rusty_acp`/`rusty_mcp` and 0.12.28 for `skillopt-model`.
  Cargo treats them as unrelated crates so they coexist cleanly and no type
  crosses between the two groups; bumping `skillopt-model` would be an API
  change outside this merge's scope. A build-size cost, not a correctness
  one.
- No dependency swaps: nothing in `rusty_skillopt` depended on a sibling in
  this workspace. Full suite (68 tests, 2 environment-gated ignores) passes
  unmodified.

## Fourth-wave merge — `rusty_inventrory`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_inventrory` — a local-first encrypted index over
  the conversation history Claude Code, Codex, Cursor, Zed, Kiro and
  Antigravity write to disk, plus its `inv` CLI and a Tauri menu-bar shell.
  Three crates behind one nested workspace, merged via `git subtree` with
  full history.
- **Changed:** its `[workspace.package]` collided with this root's on
  `rust-version`, `license` and `repository`, so its three crates carry
  literal `[package]` fields (the `rusty_db` treatment); only its
  dependencies were hoisted. `thiserror` stays literal on `inventory-core`
  for the same `"2"`-vs-`"1"` reason as `rusty_test`'s `contract`.
- **Changed:** CI's Linux leg installs `libwebkit2gtk-4.1-dev`,
  `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (the Tauri
  shell) and `libdbus-1-dev` (`inventory-core`'s Secret Service keyring),
  matching what `rusty_inventrory`'s own CI installed. Windows and macOS
  use OS-native APIs for both.
- **Fixed:** `inventory-core`'s `rusqlite = "0.37"` needs
  `libsqlite3-sys ^0.35`, which cannot coexist with `sqlx-sqlite`'s
  `^0.30.1` — `libsqlite3-sys` sets `links = "sqlite3"`, so exactly one
  version may exist per graph. Moving *up* would mean `sqlx 0.9` across
  `rusty_db` and `rusty-search-sqlite-fts5`, so `inventory-core` came down
  to `rusqlite = "0.32.1"`, unifying with `rusty_sqlite`. Verified by
  running its suite, not by reading changelogs: 79 tests pass unmodified.
- **Fixed:** three deprecated `GenericArray::from_slice` calls in `db.rs`'s
  sealed-index code — same `generic-array` 0.14.9 cause as `rusty_croc`'s,
  same behavior-preserving rewrite.
- No dependency swaps: nothing in `rusty_inventrory` depended on a sibling
  in this workspace.

## Fourth-wave merge — `rusty_test`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_test` — the `portable-runtime-contract` spike:
  one execution contract (`contract`), a per-host adapter (`compat`), a
  verification layer (`conformance`), and three reference tools
  (`stat-tool`, `proc-runner`, `pty-shell`). Merged via `git subtree` with
  full history; its nested `[workspace]` table removed and its six crates
  added to this root's `members`.
- **Changed:** its `[workspace.package]` didn't collide with this root's
  (same edition, same license), so its crates keep inheriting via
  `field.workspace = true` — the `rusty_search` treatment, not
  `rusty_db`'s. Only `publish = false` was new here. `thiserror` was
  deliberately left un-hoisted: `rusty_test` wanted `"2.0"`, this root
  pins `"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
  `thiserror = "2"` instead of forcing a major bump on unrelated crates.
- **Fixed:** `conformance`'s `tests/layering.rs` reads the workspace
  manifest to enforce the layer model, resolving it two directories above
  its own crate and requiring a declared layer for every member found.
  Post-merge that is this monorepo's root — four levels up, ~100 members —
  so three of its four tests panicked. Repointed and filtered through a
  `GROUP_PREFIX` constant; the check's logic is otherwise untouched and all
  four tests pass, alongside the group's other 27.
- No dependency swaps: nothing in `rusty_test` depended on a sibling in
  this workspace.

## Fourth-wave merge — `rusty_croc`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_croc` — a Rust port of
  [croc](https://github.com/schollz/croc), wire-compatible with stock croc
  v10 (PAKE code phrases, relay, local-network hand-off, resume). Merged
  via `git subtree` with its full commit history, as with every prior
  crate import.
- **Fixed:** four `GenericArray::from_slice` calls in `crypt.rs` (AES-256-GCM
  and XChaCha20-Poly1305 nonces). The standalone repo's lockfile pinned
  `generic-array` 0.14.7; this workspace resolves 0.14.9, which deprecates
  the crate wholesale, so `-D warnings` turned them into errors. Rewritten
  to the `From<&[T]> for &GenericArray` conversion `from_slice` delegates
  to — no behavior change, 49 tests pass unmodified.
- No dependency swaps: `rusty_croc` depends only on crates.io crates, not
  on any sibling in this workspace. Its nightly-only `fuzz/` harness keeps
  its own `[workspace]` table and is excluded from this one, same as
  `rusty_tls/fuzz` and `rusty_lsp/fuzz`.

## PR #65 — Deduplicate `rusty_rdp`'s byte cursor and split `rusty_ansder`'s two crates
**2026-09-01** · [#65](https://github.com/Rusty-Mill/rusty_mill/pull/65)

- **Fixed:** `rusty_rdp`'s hand-rolled byte Reader/Writer duplicated
  `rusty_wire`'s (a dependency `rusty_rdp` already declared but never
  used) — now re-exports `rusty_wire`'s cursor types.
- **Changed:** `rusty_ansder` bundled two unrelated libraries (an ASN.1 DER
  codec and a sovereign RAG/Q&A engine). Split the RAG engine into a new
  `rusty_rag` crate; `rusty_ansder` now holds just the DER codec.
- Also investigated and deferred (different-scoped tools sharing a name,
  not true duplication): `rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER
  codec vs. `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs.
  `rusty_url::Url`.

## PR #10 — Collapse workspace duplication: to_wide, read_lines, SHA-1, IFS splitting, glob, raw-mode
**2026-08-27** · [#10](https://github.com/Rusty-Mill/rusty_mill/pull/10)

- **Fixed:** six of eight findings from a five-sweep duplication review
  (issues #1–#8) — `rusty_win32`'s 7x-duplicated `to_wide()` hoisted;
  `rsed`/`rawk`'s shared stdin-reading extracted to `read_lines()`;
  `rusty_git`/`rusty_term`'s independent SHA-1 implementations merged into
  a new `rusty_sha1` crate; `rush`'s two independent IFS-splitting
  implementations merged into `ifs_run_end()`; `rush`'s backtracking glob
  matcher now tries `rusty_regx::Glob` first; a duplicated Windows
  raw-mode flag transformation (`rusty_term`/`rusty_lines`) hoisted into
  `rusty_win32::console::raw_mode_core()`.
- **Known limitation:** two findings closed `no action` — Unix termios
  save/restore (`rusty_term`/`rusty_lines`) is the same shape by
  deliberate, different policy; a `no_std` rounding workaround in
  `rusty_font` (`round_nonneg` vs. `round_f32`) likewise.
- Filed #9 for the remaining gap (`rusty_regx::Glob` needs embedded `!(p)`
  negation support before rush's fallback matcher can be fully deleted) —
  a capability gap, not duplication; still open as of this writing.

## Earlier crate-import history

Every `Import <crate> into crates/<crate>` merge and the CI-scoping work
(`Speed up CI: affected-crate filtering, rust-cache, nextest, parallel
clippy`) predate this file. See `git log --oneline --merges` for the full
list — not backfilled entry-by-entry here since each import is already its
own reviewable commit with a descriptive message, and there are dozens of
them (see the crate table in `README.md` for the two-wave merge history).

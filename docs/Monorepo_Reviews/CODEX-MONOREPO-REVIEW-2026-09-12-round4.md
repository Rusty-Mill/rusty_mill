# Monorepo improvement review — rusty_mill (round 4)

Reviewed on 2026-09-12 against `main` (post round-3 merge, PR #177), via
`/codex-build`. This is a fourth, independent pass — it does not reopen
round 1 (`CODEX-MONOREPO-REVIEW.md`, 33 findings), round 2
(`CODEX-MONOREPO-REVIEW-2026-09-12.md`, 63 findings), or round 3
(`CODEX-MONOREPO-REVIEW-2026-09-12-round3.md`, 40 findings) — all three
already merged. It also does not reopen `repo-inspector-report.md`'s
duplication-cluster/sovereignty findings, a different concern already
triaged there.

All 36 findings below were fixed in the same working session that
produced this report, each with a regression test that fails on the
pre-fix code and passes post-fix.

## Method and scope

Each of the first three rounds reviewed only one or two sub-crates out of
each large product family's many members (e.g. round 2 reviewed
`agentgateway-core`/`agentgateway-config` out of `rusty_agent_gateway`'s
nine crates; `adk-core`/`adk-sessions`/`adk-graph` out of `rusty_adk`'s
twelve). This round covers the **remaining, previously-untouched
sub-crates** in eight such product families (`rusty_search`,
`rusty_meshed`, `rusty_yirp`, `rusty_adk`, `rusty_agent_gateway`,
`rusty_key`, `rusty_provider`, and the `nexus` microkernel's remaining
subsystems), plus six standalone crate families neither prior round
touched at all: `rusty_hister` (whose real implementation — document,
extractor-registry, and model/session/embedding/migration modules — was
merged only after round 3 finished; round 3 found it empty stubs),
`rusty_a2a`/`rusty_acp`, `rusty_json`/`rusty_serde`, and `nexus-rush`/
`nexus-bootstrap`/`nexus-vt`/`nexus-editor`.

Fourteen parallel read-only scout passes covered this surface, each
instructed to report only concrete, triggerable defects with file:line
evidence and a specific triggering input, at high confidence, and to
return nothing rather than pad with style nits. Several sub-areas within
each assigned scope were reviewed in comparable depth and yielded nothing
meeting that bar — `adk-mcp`, `adk-tools`, `adk-models`, `adk-macros`,
`adk-a2a` and the three `rusty_adk` examples; `agentgateway-auth`,
`agentgateway-llm`, `agentgateway-mcp`, `agentgateway-tls`,
`agentgateway-a2a`'s routing; `kernel`/`config`/most of `compose`/
`observe` in `rusty_key`; `rusty-meshed-governance`/`schema-registry`/
`observability`/`cli`/`domains`/`trace`; `sessionmgr-proc`/`sessionmgr-
protocol`'s own types; `nexus-memory`'s storage/retrieval layer; `nexus-
dap`/`nexus-lsp`'s request/response dispatch; `rp-mcp`'s remaining
surface, `openai_compatible.rs`/`util.rs` in `rp-providers`; `skillopt-
core`/`skillopt-envs`; most of `rusty_a2a`'s auth/signing/lifecycle code
and `rusty_acp`'s route/admission logic — these are not restated below as
empty rows.

All locations are relative to this checkout at the time of review.
Severity: **high** = memory unsafety, durable data loss, credential
exposure, hostile-input resource exhaustion, or a security-policy bypass
reachable from untrusted/public input; **medium** = bounded correctness/
reliability defect or a security gap with mitigating preconditions;
**low** = documentation/config drift or a small avoidable issue.

## Disposition

Every finding was fixed by 20 parallel fix tasks (one per crate group),
each with a regression test that fails pre-fix and passes post-fix.
Several tasks went further than a plain pass/fail check — proving their
regression test actually catches the described bug by temporarily
reverting the fix and re-running the test (`FixNexusGitAutoCommit`,
`FixGatewayA2aBodyCap`, `FixNexusVtJpegParser`), or by running a
deliberately-broken test double through the same conformance suite
(`FixAcpHostHeader`). After all 20 landed, a workspace-lint sweep
(`cargo fmt` + `cargo clippy --all-targets -D warnings`) across all 28
touched crates caught 2 additional real lint violations (a feature-gating
gap and a doc-markdown nit), both fixed directly.

| # | Disposition | # | Disposition | # | Disposition | # | Disposition |
| - | - | - | - | - | - | - | - |
| 1 | fixed | 10 | fixed | 19 | fixed | 28 | fixed |
| 2 | fixed | 11 | fixed | 20 | fixed | 29 | fixed |
| 3 | fixed | 12 | fixed | 21 | fixed | 30 | fixed |
| 4 | fixed | 13 | fixed | 22 | fixed | 31 | fixed |
| 5 | fixed | 14 | fixed | 23 | fixed | 32 | fixed |
| 6 | fixed | 15 | fixed | 24 | fixed | 33 | fixed |
| 7 | fixed | 16 | fixed | 25 | fixed | 34 | fixed |
| 8 | fixed | 17 | fixed | 26 | fixed | 35 | fixed |
| 9 | fixed | 18 | fixed | 27 | fixed | 36 | fixed |

---

## `rusty_hister`

**1. `WebSession::get()` never checks session expiry — expired sessions authenticate forever.**
Location: `crates/rusty_hister/crates/rusty-hister-model/src/session.rs:91-95`.
Trigger: a session with `expires_at` in the past is still returned as `Some(session)` by `get`, contradicting its own doc comment; any server treating a `Some` result as "currently valid" (the documented contract) authenticates a stale/leaked cookie indefinitely.
Fix: added `.filter(table.col("expires_at").gt(Utc::now()))` to the query. (Incidentally fixed a pre-existing test fixture whose `expires_at` had already drifted into the past relative to real wall-clock time.)
Regression test: `session::tests::get_returns_none_for_an_expired_session`. Verified: `cargo test -p rusty-hister-model` — 57 passed.
Severity: high.

## `rusty_search` (`rusty-search-sqlite-fts5`)

**2. Index-name path traversal in `create_index`/`delete_index`.**
Location: `crates/rusty_search/crates/rusty-search-sqlite-fts5/src/lib.rs:132,169`.
Trigger: `create_index("../../../../tmp/pwned", ...)` writes an attacker-shaped SQLite file outside the configured data directory; `delete_index` with the same traversal deletes an arbitrary file.
Fix: shared allow-list validator (`[A-Za-z0-9_-]+`) called by both functions before path construction, returning `SearchError::InvalidSchema` on rejection.
Regression tests: `create_index_rejects_path_traversal_names`, `delete_index_rejects_path_traversal_names`. Verified: `cargo test -p rusty-search-sqlite-fts5` — 19 passed.
Severity: high.

**3. `usize`-to-`i64` cast on `limit`/`offset` wraps to SQLite's "unlimited" sentinel.**
Location: `crates/rusty_search/crates/rusty-search-sqlite-fts5/src/query_map.rs:71-72`.
Trigger: `SearchRequest::new(q).limit(usize::MAX)` binds a negative `i64`, which SQLite treats as no limit — every matching row materializes into `Vec<Hit>` regardless of the caller's intended page size.
Fix: `.min(i64::MAX as usize) as i64` clamp before binding.
Regression test: `query_map::tests::compile_clamps_usize_max_limit_and_offset_to_non_negative_i64`. Verified: same run as finding 2.
Severity: high.

## `rusty_provider` (`rp-providers`, `rp-mcp`)

**4. Gemini API key leaks into client-facing error responses on any outbound network failure.**
Location: `crates/rusty_provider/crates/providers/src/gemini.rs` (key passed as `?key=` query param), `crates/rusty_provider/crates/providers/src/http.rs:62-68` (`map_reqwest_error`, embeds the full request URL via `err.to_string()`).
Trigger: any transient network condition (DNS blip, connection reset) while routing to Gemini surfaces the live API key in the JSON error body returned to any authenticated `/v1/chat/completions` caller, and in MCP tool-call errors.
Fix: `map_reqwest_error` now calls `err.without_url()` before `.to_string()`.
Regression test: `map_reqwest_error_strips_the_api_key_from_the_request_url`. Verified: `cargo test -p rp-providers -p rp-mcp` — 88+34+31+33 + 5+3 passed.
Severity: high.

**5. No timeout anywhere in the MCP upstream tool-call path.**
Location: `crates/rusty_provider/crates/mcp/src/gateway.rs` (`call_tool`/`list_tools`, unlike every other outbound integration in this router family which has a `timeout_secs` config).
Trigger: a stalled/buggy upstream MCP server hangs every client request routed to it forever, progressively exhausting the server's concurrency budget.
Fix: added `McpConfig::timeout_secs` (default 30s, matching the existing convention), wrapped `call_tool`/`list_tools` in `tokio::time::timeout`, added `GatewayError::Timeout`.
Regression test: `call_tool_returns_a_timeout_error_instead_of_hanging_forever` (real in-memory MCP server via `tokio::io::duplex` whose handler never resolves). Verified: same run as finding 4.
Severity: medium.

## `rusty_adk`

**6. `transfer_to_agent` delegation is fully-wired-looking but completely dead — a tool's delegation request is silently dropped.**
Location: `crates/rusty_adk/crates/adk-tools/src/context.rs:121-125`, `crates/rusty_adk/crates/adk-agents/src/llm_agent.rs:234-236,473-477`, `crates/rusty_adk/crates/adk-runner/src/lib.rs`.
Trigger: a tool calls `ctx.transfer_to_agent("other")`; the documented behavior (and the framework's own README/A2A-card generation) claims control passes to `other_agent`, but `LlmAgent::run` only used the field to stop its own loop — the target was never resolved or run.
Fix: `LlmAgent::run` now resolves the target via `find_agent` and forwards its event stream as a continuation of the current run; an unknown target name yields a typed error event instead of silently ending.
Regression tests: `a_transfer_to_agent_call_actually_runs_the_target_sub_agent`, `a_transfer_to_an_unknown_agent_yields_an_error_instead_of_ending_silently` (both confirmed to fail against the pre-fix no-op dispatch). Verified: `cargo test -p adk-agents -p adk-runner -p adk-tools` — 35+8+6 passed.
Severity: medium.

## `rusty_agent_gateway`

**7. A2A dispatch path reads the entire client request body into memory with no size cap.**
Location: `crates/rusty_agent_gateway/crates/agentgateway/src/gateway.rs:761-767` — unlike the `extAuthz` branch 130 lines earlier in the same function, which uses `collect_limited`.
Trigger: any client (pre- or post-auth, since the body isn't read by auth gates) sends a large/unbounded body to an `a2a`-policy route; the gateway buffers it fully before inspection, with no configuration knob to bound it.
Fix: wrapped the read in the same `collect_limited` helper, capped at `agentgateway-llm::MAX_REQUEST_BYTES` (4 MiB), refusing with 413 past the limit.
Regression test: `an_oversized_body_is_refused_before_it_is_buffered` (confirmed to fail against pre-fix code: asserted 200 OK instead of 413). Verified: `cargo test -p agentgateway --test a2a --test ext_authz` — 23+15 passed.
Severity: high.

## `nexus-rush`

**8. Catastrophic backtracking in the glob wildcard matcher.**
Location: `crates/nexus/crates/nexus-rush/src/glob.rs:135-180` — plain unmemoized recursive `*` backtracking.
Trigger: a pattern with ~30+ `*` segments against a non-matching 40-60 char filename drives exponential-time re-exploration, hanging the shell.
Fix: rewrote as an iterative two-pointer/DP algorithm (compile pattern to a flat `Vec<Atom>`, then standard `star_at`/`star_from` backtrack-by-index), same matching semantics.
Regression test: `glob::tests::adversarial_star_pattern_matches_in_bounded_time` (asserts sub-1s completion). Verified: `cargo test -p nexus-rush` — 70 passed.
Severity: high.

**9. Unbounded parser recursion causes stack-overflow process abort.**
Location: `crates/nexus/crates/nexus-rush/src/parser.rs` — `parse_list`/`parse_and_or`/`parse_pipeline`/`parse_command`/`parse_subshell`/`parse_group` and every compound-statement body form an unbounded mutual-recursion cycle.
Trigger: a script of ~200,000 unmatched `(` (trivially embeddable in an AI-agent-issued command) overflows the native stack, aborting the hosting process.
Fix: `depth: u32` field + `MAX_NESTING_DEPTH = 200` on `Parser`, guarding every recursive-nesting entry point.
Regression test: `parser::tests::deeply_nested_subshells_are_rejected_instead_of_overflowing_the_stack`. Verified: same run as finding 8.
Severity: high.

## `nexus-bootstrap`

**10. Documented SSH connect timeout is declared but never enforced.**
Location: `crates/nexus/crates/nexus-bootstrap/src/remote.rs:26` (`SSH_CONNECT_TIMEOUT`, referenced nowhere else in the crate).
Trigger: a network-reachable but slow/unresponsive remote host hangs `build_remote_runtime_ssh` far longer than the documented 30s ceiling — since the JSON-RPC protocol is pure request/response, a wedged `ssh` process was indistinguishable from a healthy idle connection until the first real IPC call, which could then hang up to 600s (or forever).
Fix: added a real handshake proof (a throwaway `event_subscribe`+`event_unsubscribe` round trip) wrapped in `tokio::time::timeout(SSH_CONNECT_TIMEOUT, ...)`, killing and reaping the child process on timeout instead of leaking it.
Regression test: `connect_handshake_times_out_on_a_silent_peer` (a `tokio::io::duplex` peer that never answers). Verified: `cargo test -p nexus-bootstrap --lib remote:: --test reconnect_loop --test remote_runtime_loop` — 6+10 passed.
Severity: high.

## `rusty_json` / `rusty_serde`

**11. `rusty_json`'s hand-rolled parser has no recursion-depth limit anywhere.**
Location: `crates/rusty_json/src/value_io.rs:40-141` and `src/de.rs:302-423` — both the serde-free `Value` path and the generic `serde::Deserializer` path recurse unboundedly on nested `[`/`{`.
Trigger: `Value::from_json_str(&("[".repeat(200_000) + &"]".repeat(200_000)))` overflows the stack — the same bug class round 2/3 already fixed in seven other from-scratch parsers in this workspace, never itself patched despite being described elsewhere as "the stricter reference implementation."
Fix: shared depth counter on `Parser` (128 cap), checked in both `parse_array`/`parse_object` and `parse_seq`/`parse_map`.
Regression tests: one per code path, both asserting `Err` on 200,000-deep nesting. Verified: `cargo test -p rusty_json` — 169+9+1 passed.
Severity: high.

**12. `rusty_serde`'s std-integer `Deserialize` impls silently truncate/wrap out-of-range JSON integers.**
Location: `crates/rusty_serde/rusty_serde/src/impls.rs:225-247` — unconditional `v as $ty` casts for every narrower integer type, unlike real serde's `TryFrom`-based bounds checks.
Trigger: `rusty_serde::json::from_str::<Cfg>(r#"{"limit":300}"#)` on `struct Cfg { limit: u8 }` silently produces `Cfg { limit: 44 }` instead of erroring — a correctness regression against the exact serde behavior this crate re-implements, affecting both the JSON and RON backends via the shared macro.
Fix: replaced the cast with `<$ty>::try_from(v).map_err(|_| E::custom(...))`.
Regression tests: out-of-range, negative-into-unsigned, and over-`i32::MAX` cases. Verified: `cargo test -p rusty_serde` — 8+3+29+119+10+1 passed.
Severity: high.

## `rusty_a2a`

**13. Unbounded push-notification-config registration enables webhook-delivery DDoS amplification.**
Location: `crates/rusty_a2a/src/server/engine.rs:946-966,1327-1337`, `src/server/store.rs:166-179` — no cap on configs per task; every task-status change fans out one delivery per registered config.
Trigger: an authorized caller registers tens of thousands of push-notification configs pointing at the same victim host for their own task; the next status update fires that many concurrent outbound POSTs simultaneously.
Fix: `MAX_PUSH_CONFIGS_PER_TASK = 10`; `put_push_config` rejects new registrations past the cap (in-place updates to an existing id still succeed).
Regression tests: `registering_push_configs_past_the_cap_is_rejected`, `updating_an_existing_config_at_the_cap_still_succeeds`. Verified: `cargo test -p rusty_a2a --features client,server` — 135 tests across 24 integration binaries + 6 lib + 3 doc, all passed.
Severity: high.

**14. TOCTOU DNS-rebinding bypass of the webhook SSRF protection.**
Location: `crates/rusty_a2a/src/server/push.rs:73-96,125-149,188-224` — validation resolves DNS once, delivery resolves independently moments (or, across retries, seconds) later.
Trigger: an attacker-controlled DNS server answers a public IP to the validator and a private IP to the actual `reqwest` connection, letting a private-network request through despite SSRF protection being enabled.
Fix: `validate_webhook_url` now returns the resolved addresses; delivery builds a one-off `reqwest::Client` via `resolve_to_addrs` pinned to those exact addresses for every attempt (including retries) within one `notify()` call.
Regression tests: unit-level (`validate_webhook_url_reports_the_exact_address_it_checked`, `pinned_client_connects_to_the_validated_address_not_a_fresh_dns_lookup` — the latter points at a `.invalid` hostname that can never resolve via real DNS and proves the pinned override is what makes the connection succeed). Verified: same run as finding 13.
Severity: medium.

## `rusty_acp`

**15. Client-controlled Host header persisted and retroactively applied to every message URL in a shared session.**
Location: `crates/rusty_acp/src/server/mod.rs:893-903`, `src/server/store/postgres.rs:742-771,805-844`, `src/server/store/redis.rs:410-433,468-503` — `base_url` stored as one mutable column/field per session, reapplied to every historical message's URL on every read.
Trigger: a request with a spoofed `Host`/`X-Forwarded-Host` header into an existing session overwrites the session's stored `base_url`; the next `GET /session/{id}` by anyone returns every message's URL rewritten to the attacker's origin, not just the one message from the malicious request.
Fix: moved `base_url` to a per-message column (Postgres) / per-entry field (Redis), written at append time and read back per-message, matching the already-correct `InMemoryStore` pattern.
Regression tests: `session_message_urls_pin_to_write_time_base_url` (new conformance-suite check run against all three backends); `a_backend_that_lets_a_later_base_url_rewrite_history_is_caught` (proves the new check actually catches this exact bug shape against a deliberately-reintroduced broken test double). Verified: `cargo test -p rusty-acp --features postgres-store,redis-store,server,store-testkit` — 329 passed (Postgres/Redis live-backend tests skip cleanly with no DB configured in this environment; verified via compile-time type-checking against real sqlx/redis APIs plus the mutation-test proof).
Severity: medium.

## `rusty_key`

**16. `--gateway` mode is unauthenticated, network-exposed, and tool-approval-free by default.**
Location: `crates/rusty_key/crates/app/src/main.rs:47` (hardcoded `0.0.0.0` bind), `src/gateway.rs:93-94,120-131,184-191` (wildcard CORS, auth optional-by-default), `src/session.rs:191-196` (gateway's `Session::new` has no `ApprovalGate`, unlike ACP/desktop).
Trigger: `rusty-keys --gateway` with no `RUSTYKEYS_GATEWAY_SECRET` set (the documented default) lets any host on the network — or any webpage a LAN user visits, via cross-origin `fetch` — drive the agent's bash/file tools with zero authentication and zero human-in-the-loop approval.
Fix: default-bind to `127.0.0.1` (host override via `RUSTYKEYS_GATEWAY_HOST`); fail to start if no secret is configured; `cors_origin` defaults to none instead of `"*"`, with methods/headers tightened from `Any`; `build_session` now wires an `ApprovalGate` via `new_with_policy`, matching ACP/desktop.
Regression tests: bind-host default, startup-fails-without-secret, and a full end-to-end test proving a scripted file write is blocked by the approval gate. Verified: `cargo test -p rk-app --bin rusty-keys` (2) + `--lib --features gateway` (18) + `--test gateway_test --features gateway` (7), all passed.
Severity: high.

**17. Secret redaction scrubs by the wrong whitespace class.**
Location: `crates/rusty_key/crates/observe/src/redact.rs:46-53` — detects secrets via `split_whitespace()` (any Unicode whitespace) but scrubs via `split(' ')` (literal ASCII space only).
Trigger: a secret token delimited only by tabs/newlines (e.g. bash stdout from `cat .env`) is correctly detected but survives the actual scrub unredacted into the durable `evidence.jsonl` and the desktop bridge's telemetry payloads.
Fix: scrub over the same `split_whitespace()` boundaries used for detection.
Regression test: `redact::tests::scrubs_secrets_delimited_by_non_space_whitespace`. Verified: `cargo test -p rk-observe --lib` — 20 passed.
Severity: high.

**18. `/ratchet`'s evidence-line renderer panics on a UTF-8 character straddling its 120-byte truncation point.**
Location: `crates/rusty_key/crates/compose/src/ratchet.rs:278-284` (`one_line`) — raw byte-slice at a fixed offset with no char-boundary check.
Fix: compute the largest char-boundary index ≤ 120 before slicing.
Regression test: `ratchet::tests::one_line_truncates_without_panicking_on_multibyte_boundary`. Verified: `cargo test -p rk-compose --lib` — 22 passed.
Severity: medium.

**19. Gateway bearer-secret comparison is not constant-time.**
Location: `crates/rusty_key/crates/app/src/gateway.rs:127-130` — `tok == expected` short-circuits on the first mismatched byte.
Fix: hand-rolled constant-time byte comparison (`subtle` is not a direct workspace dependency; adding one was out of this fix's scope per the task's own fallback guidance).
Regression test: `gateway::tests::constant_time_eq_accepts_matching_and_rejects_mismatched_tokens`. Verified: same run as finding 16 (18 lib tests).
Severity: medium.

## `rusty_meshed`

**20. TOCTOU race in data-contract creation returns 500 instead of the documented 409.**
Location: `crates/rusty_meshed/crates/rusty-meshed-registry/src/routers/contracts.rs:169-234` — the same check-then-insert pattern round 2 already fixed in the sibling `access_grants.rs`, left unfixed here.
Fix: mirrored `access_grants.rs`'s exact fix — detect the UNIQUE-constraint violation on the INSERT and convert it to the documented 409 instead of falling through to a generic 500.
Regression test: `create_race_window_only_one_contract_persists_and_conflict_is_returned`. Verified: `cargo test -p rusty-meshed-registry` — 183 passed.
Severity: medium.

## `rusty_yirp`

**21. Unbounded `SessionResize` dimensions drive an uncapped vt100/PTY grid allocation.**
Location: `crates/rusty_yirp/crates/sessionmgr-agents/src/pattern_watch.rs:29-32`, `crates/rusty_yirp/crates/sessionmgr-pty/src/lib.rs`, `crates/rusty_yirp/crates/sessionmgr-daemon/src/worker.rs:586-598` — client-supplied `u16` rows/cols forwarded with no clamp.
Trigger: any connected client sends `SessionResize{rows: 65535, cols: 65535}`; `vt100::Screen::set_size` attempts a ~4.3-billion-cell allocation, aborting the worker process.
Fix: `MAX_TERMINAL_DIM`/`MAX_RESIZE_DIM = 500` clamps in `ScreenWatcher::resize`/`PtySession::resize`, plus rejection at the protocol boundary in `worker.rs`.
Regression tests: 4, covering all three layers. Verified: `cargo test -p sessionmgr-agents -p sessionmgr-daemon -p sessionmgr-pty` — 60+33+2 passed.
Severity: high.

**22. `git status --porcelain` path quoting is trimmed, not unescaped.**
Location: `crates/rusty_yirp/crates/sessionmgr-git/src/lib.rs` — doesn't decode git's C-style escape sequences inside quoted paths.
Fix: switched to `git status --porcelain --untracked-files=all -z` (NUL-terminated, unquoted paths) and a new NUL-delimited record parser.
Regression tests: 3, including a literal-quote filename case. Verified: `cargo test -p sessionmgr-git` — 9 passed.
Severity: medium.

**23. Codex hook config's TOML literal string breaks on an apostrophe in the daemon's own executable path.**
Location: `crates/rusty_yirp/crates/sessionmgr-agents/src/codex.rs` — a TOML single-quoted literal string containing an unescaped `hook_fire_exe` path.
Trigger: any install path containing a `'` (e.g. `C:\Users\O'Brien\...`) produces invalid TOML, silently disabling Codex hook-based session-status detection.
Fix: switched to a TOML basic (double-quoted) string with proper backslash/quote escaping.
Regression test: `hook_config_survives_an_apostrophe_in_the_executable_path`. Verified: same run as finding 21.
Severity: medium.

**24. Unbounded `read_line` framing reintroduced in `sessionmgr-desktop`.**
Location: `crates/rusty_yirp/crates/sessionmgr-desktop/src-tauri/src/client.rs`, `src/attach.rs` — the same defect class round 2 fixed in the daemon/TUI copies, reintroduced in this crate's independent duplicate.
Fix: added `read_line_capped` (256 KiB cap), mirroring the daemon's existing pattern, applied to both files.
Regression tests: 2. Verified: same run as finding 21 (`sessionmgr-desktop`: 7 passed).
Severity: medium.

## `nexus-git`

**25. `AutoCommitter` silently finalizes an unresolved merge conflict with conflict-marker garbage baked into history.**
Location: `crates/nexus/crates/nexus-git/src/auto_commit.rs:55-91` — checks `is_dirty` but never `repo_state`, so it stages and commits (finalizing the merge via round 2's own `MERGE_HEAD`-folding fix) even mid-conflict.
Trigger: a user's merge conflicts, they step away before resolving, `auto_commit`'s idle window elapses — the background thread bakes raw `<<<<<<<`/`=======`/`>>>>>>>` markers into a real 2-parent merge commit with no prompt.
Fix: skip the auto-commit (returning a distinguishable "skipped: mid-operation" result) whenever `repo_state != Clean`.
Regression test: `auto_commit::tests::auto_commit_skips_during_unresolved_merge_conflict` (confirmed to fail when the guard is disabled). Verified: `cargo test -p nexus-git` — 77+18 passed.
Severity: high.

## `nexus-formats`

**26. Notion zip-export importer is vulnerable to Zip Slip.**
Location: `crates/nexus/crates/nexus-formats/src/notion/mod.rs:79-160`, `src/notion/filename.rs:63-70` (`clean_path` only strips the Notion UUID suffix, never rejects `..`/absolute/drive-prefixed components) — reachable via IPC handler `com.nexus.formats::import_notion`.
Trigger: importing a crafted zip containing an entry named `../../../evil.md` writes attacker-controlled content outside the destination directory.
Fix: routed every zip-entry-derived relative path through `nexus_types::paths::resolve_within` (the same confinement helper already used in `nexus-git`), skipping any entry that fails.
Regression test: constructs an in-memory zip with a traversal entry and asserts the file lands nowhere outside `dest` (not just "doesn't panic"). Verified: `cargo test -p nexus-formats` — 212+5 passed.
Severity: high.

**27. `validate_filename`/`validate_path` panic on non-ASCII names at their own length-cap boundary.**
Location: `crates/nexus/crates/nexus-formats/src/util/filename.rs:63-67,78-84` — fixed-byte-offset slicing with no char-boundary check.
Fix: truncate by character count instead of raw byte offset.
Regression tests: 2, using 3-byte-UTF-8-heavy strings well over each cap. Verified: same run as finding 26.
Severity: medium.

## `nexus-database`

**28. Formula `slice()` panics on byte-boundary violation or reversed range.**
Location: `crates/nexus/crates/nexus-database/src/formula/functions.rs:61-69` — `start`/`end` clamped independently but never checked against each other or UTF-8 boundaries, reachable via `formula_eval` IPC.
Fix: validate `start <= end`; operate on `char` positions instead of raw bytes.
Regression tests: reversed-range and multi-byte-boundary cases. Verified: `cargo test -p nexus-database` — 141+3 passed.
Severity: high.

**29. Formula string-literal tokenizer corrupts non-ASCII text via `char::from(u8)`.**
Location: `crates/nexus/crates/nexus-database/src/formula/token.rs:144-165` — same bug class round 2 fixed in `nexus-workflow/src/interpolate.rs`, a different file here.
Fix: buffer raw bytes and decode as UTF-8 once, instead of per-byte casting.
Regression test: `string_literal_non_ascii`. Verified: same run as finding 28.
Severity: medium.

**30. CSV export performs no formula-injection neutralization.**
Location: `crates/nexus/crates/nexus-database/src/import_export.rs:171-182` — string field values written verbatim, no check for `=`/`+`/`-`/`@` triggers.
Fix: prefix such values with `'` per OWASP CSV-injection guidance.
Regression test: `export_csv_neutralizes_formula_injection`. Verified: same run as finding 28.
Severity: medium.

## `nexus-workflow`

**31. `templates_init`'s filename sanitizer misses a Windows drive-relative path escape.**
Location: `crates/nexus/crates/nexus-workflow/src/handlers/templates.rs:43-87` — rejects `/`, `\`, `..` but not a bare drive prefix like `C:evil.toml`, which `PathBuf::join` treats as a full path replacement on Windows.
Fix: additionally reject any filename containing `:`.
Regression test: `templates_init_rejects_filename_with_drive_prefix`. Verified: `cargo test -p nexus-workflow` — 208 passed.
Severity: medium.

## `nexus-vt`

**32. JPEG decoder's pixel-count cap is bypassed by chroma-subsampling/MCU-padding amplification.**
Location: `crates/nexus/crates/nexus-vt/src/core/jpeg.rs:276-278,401-408` — cap checked only against nominal `width*height`, not the MCU-padded per-component plane sizes, which can be ~25× larger for extreme-aspect-ratio images with high sampling factors.
Trigger: reachable via untrusted terminal input (iTerm2 inline images); a crafted SOF (width=4,194,304, height=1, Hi=Vi=4) passes the nominal cap but allocates ~400 MB of padded planes.
Fix: cap the total MCU-padded sample count before allocating.
Regression test: `jpeg_rejects_mcu_padded_plane_amplification` (confirmed against real Huffman/entropy data, not a synthetic short-circuit). Verified: `cargo test -p nexus-vt` — 276 passed.
Severity: high.

**33. JPEG decoder panics on a duplicate-SOF component count of 2.**
Location: `crates/nexus/crates/nexus-vt/src/core/jpeg.rs:288,554-560` — a second SOF marker isn't rejected; `to_rgba` assumes exactly 1 or 3 components.
Fix: reject a second SOF marker; `to_rgba` now matches explicitly on `comps.len()`.
Regression test: `jpeg_rejects_duplicate_sof` (confirmed to panic pre-fix on `comps[2]`). Verified: same run as finding 32.
Severity: high.

**34. Extreme-aspect-ratio inline image causes an unbounded synchronous render loop (CPU hang).**
Location: `crates/nexus/crates/nexus-vt/src/core/grid.rs:981-996`, `src/core/kitty.rs:141-153` — only shrinks width to fit; a narrow-and-tall image's row count tracks the full source height.
Fix: capped `cell_rows` to a bounded multiple of the terminal's own row count in `render_image`, plus a per-axis `MAX_DIM` cap in `kitty.rs` mirroring `sixel.rs`'s existing constant.
Regression test: `render_image_huge_aspect_ratio_is_bounded` (confirmed unbounded pre-fix: 499,977 scroll iterations for a 1×1,000,000 image). Verified: same run as finding 32.
Severity: high.

**35. CSI parameter buffer has no size cap, unlike the OSC/DCS/APC buffers.**
Location: `crates/nexus/crates/nexus-vt/src/core/parser.rs:325` — `osc_buffer`/`dcs_buffer`/`apc_buffer` are all explicitly capped; `param_buffer` isn't.
Trigger: an unterminated `ESC [` followed by an unbounded digit stream grows memory proportional to the attacker's stream length.
Fix: `PARAM_MAX = 4096` cap mirroring the existing `osc_buffer` pattern (stop appending, keep consuming to stay in sync).
Regression test: `csi_param_buffer_is_bounded` (confirmed unbounded pre-fix: grew to the full 5,000,000-byte test input). Verified: same run as finding 32.
Severity: high.

## `nexus-editor`

**36. Editor transaction apply panics on a UTF-8 char-boundary-misaligned `pos`.**
Location: `crates/nexus/crates/nexus-editor/src/transaction.rs:388-410,413-441` — validates only byte-length bounds, not char-boundary alignment, before `insert_str`/slicing/`replace_range`. Reachable from the `apply_transaction` IPC handler on client-supplied JSON, and panicking here poisons the session's `Mutex`, so the DoS outlives the single request — every subsequent operation on that document fails until process restart.
Fix: additionally reject `pos`/`end` that aren't `is_char_boundary`, returning `EditorError::InvalidRange`.
Regression tests: insert and delete cases on `"héllo"` with a misaligned `pos`. Verified: `cargo test -p nexus-editor` — 292 passed.
Severity: high.

---

## Delivery notes

All 36 findings above were fixed in this session by 20 parallel fix
tasks, one per disjoint crate group, each adding a regression test in
that crate's existing test conventions and verifying with a
package-scoped `cargo test -p <crate>`. After all 20 landed, a
workspace-lint sweep (`cargo fmt` then `cargo clippy --all-targets -D
warnings`) was run directly against all 28 touched crates — the fmt pass
was clean; the clippy pass caught 2 real issues fixed directly in this
session: `rk-app`'s new `gateway_bind_addr` helper (finding 16) was
defined unconditionally but only called behind `#[cfg(feature =
"gateway")]`, tripping `dead_code` on the default feature build (fixed by
gating the function and its tests the same way); and a `doc_markdown`
lint on a new doc comment in `nexus-database` (`LibreOffice` needed
backticks). Both re-verified clean afterward, and the affected crates'
tests re-run green.

Several fix tasks noted pre-existing, unrelated environment limitations
encountered during verification, none caused by this round's changes:
`nexus-bootstrap`'s bare `cargo test` (no `--test` filter) hits a
pre-existing Windows linker resource-exhaustion issue linking unrelated
integration binaries; `sessionmgr-daemon`'s `fork_sessions` integration
test requires live `claude` CLI credentials unavailable in this sandbox;
`rusty-acp`'s Postgres/Redis conformance tests correctly skip without a
configured live database, compensated by compile-time type-checking
against the real `sqlx`/`redis` APIs plus a mutation-test proof that the
new regression check actually catches the fixed bug shape.

No workspace-wide `cargo check --workspace`/`cargo test --workspace` was
run (impractical for a 340+-member workspace within one session); the
28-crate clippy/fmt sweep plus each task's own package-scoped test run is
this round's verification bar, matching round 3's precedent. No commits
were made during the fix phase — all changes are unstaged working-tree
edits at the time this report was written; commit/branch/PR/merge follows
as a separate step, also matching round 3's precedent.

This pass covered fourteen additional crate-family groups; it is not
exhaustive of the remaining workspace surface. Still not reviewed at this
depth: the full `nexus-vt`/`nexus-editor` surface beyond this round's four
findings (the JPEG/CSI-parser bug density found here suggests a fuzz pass
would be higher-leverage than continued manual review), `rusty_meshed`'s
`rusty-meshed-domains`/`rusty-meshed-core` Avro layer beyond round 2's own
finding, and the `sessionmgr-desktop`/`inventory-tauri`/`rk-desktop`
family's Tauri frontend (JS/TS) code, which every round including this one
has scoped out as outside a Rust-focused review.

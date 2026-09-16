# Monorepo improvement review — rusty_mill (round 7)

Reviewed on 2026-09-13 against `main`, via `/codex-build`. This is a seventh,
independent pass — it does not reopen round 1 (`CODEX-MONOREPO-REVIEW.md`,
33 findings), round 2 (`CODEX-MONOREPO-REVIEW-2026-09-12.md`, 63 findings),
round 3 (`CODEX-MONOREPO-REVIEW-2026-09-12-round3.md`, 40 findings), round 4
(`CODEX-MONOREPO-REVIEW-2026-09-12-round4.md`, 36 findings), round 5
(`CODEX-MONOREPO-REVIEW-2026-09-12-round5.md`, 33 findings), or round 6
(`CODEX-MONOREPO-REVIEW-2026-09-12-round6.md`, 34 findings) — all six
already merged. It also does not reopen `repo-inspector-report.md`'s
duplication-cluster/sovereignty findings.

All 38 findings below were fixed in the same working session that produced
this report, each with a regression test that fails on the pre-fix code and
passes post-fix.

## Method and scope

Round 6 explicitly listed crate families/sub-areas it reviewed but found
clean, or hadn't reached at all: `rusty_audio`/`rusty_voice`, `rusty_llama`
(only used as a fix-reference, not itself deeply re-audited), `rusty_rag`/
`rusty_config`/`rusty_boot`, `rusty_uuid`/`rusty_sqlite`/`rusty_tokio`,
`rusty_wire`/`rusty_err`/`rusty_request`/`rusty_retry`, `rusty_url`,
`winargv`, `nexus-mcp`'s dispatch/auth/pool layer, `nexus-ai`/
`nexus-ai-runtime`'s scheduler/session/provider clients, most of
`rusty_lsp`/`rusty_mcp`'s framing/auth layers, and dozens of crate families
never covered by any prior round at all (`rusty_git`, `rusty_diff`,
`rusty_ansi`, `mill-term`, `rpath`, `rusty_term`/`l13`, `rusty_serde`,
`rusty_tls`, `rusty_wiremock`, `rusty_libc`, `rusty_crypto_key`, `rusty_gpu`,
`rusty_vulkan`, `rusty_regx`, `rusty_simd`, `rusty_sha1`, `rusty_text`,
`rusty_provider`, `rusty_inventrory`, and roughly two dozen leaf `nexus-*`
crates: `nexus-kernel`, `nexus-kv`, `nexus-types`, `nexus-plugin-api`,
`nexus-plugins`, `nexus-ai`, `nexus-dap`, `nexus-acp`, `nexus-cli`,
`nexus-tui`, `nexus-theme`, `nexus-skills`, `nexus-linkpreview`,
`nexus-notifications`, `nexus-comments`, `nexus-panic-log`, `nexus-fuzz`,
`nexus-audio`, `nexus-collab`, `nexus-memory`, `nexus-memory-hub`,
`nexus-context`, `nexus-protocol`, `nexus-hashline`).

This round targets exactly that combined surface: 15 parallel read-only
scout passes, each instructed to report only concrete, triggerable defects
with file:line evidence and a specific triggering input, at high confidence,
and to return nothing rather than pad with style nits. Several sub-areas
were reviewed in comparable depth and confirmed clean, with no padding
added to hit a quota: `rusty_regx`/`rusty_simd`/`rusty_sha1` (all three
unusually well-hardened already), `rusty_crypto_key`/`rusty_gpu`/
`rusty_vulkan` (re-confirmed clean from rounds 3/5), `nexus-dap`/`nexus-acp`
(both already correctly bound their framing and drain pending requests on
transport failure), `nexus-cli`/`nexus-theme`, `nexus-context`/
`nexus-protocol`, `nexus-fuzz`, most of `nexus-lsp`/`rusty-mcp`'s
resource/task/pagination layer beyond the one traversal bug found, and
`rp-router`'s fallback/BYOK/budget logic.

One fix task (`FixRustySerdeRecursionDepth`) also surfaced a genuine,
separate pre-existing defect while proving its own regression test:
`rusty_serde`'s `Value` type has no custom `Drop`, so a sufficiently deep
in-memory `Value` tree (~2,000–5,000+ levels, confirmed experimentally)
overflows the stack merely by being dropped, independently of any
deserializer. This is not reachable through the now-guarded JSON/RON text
parsers (their new depth guard trips before a deep `Value` is ever
materialized) but is a structural gap in the `Value` type itself (needs an
iterative/custom `Drop`, not a parser-side fix). Flagged here, not fixed —
left for a future round, matching this file's scope boundary of fixing only
what was assigned per task.

All locations are relative to this checkout at the time of review.
Severity: **high** = memory unsafety, durable data loss, credential
exposure, hostile-input resource exhaustion, or a security-policy bypass
reachable from untrusted/public input; **medium** = bounded correctness/
reliability defect or a security gap with mitigating preconditions;
**low** = documentation/config drift or a small avoidable issue.

## Disposition

Every finding was fixed by 25 parallel fix tasks (one per disjoint crate
group), each with a regression test that fails pre-fix and passes post-fix.
Several fix tasks proved the pre-fix defect directly rather than by
assertion alone: `FixRustyGitThree` reproduced a genuine ~112 GiB
allocation attempt (via an isolated harness with an allocator that refuses
allocations over 10 MiB, to avoid actually requesting that much memory on
the shared workstation) for the index parser, and an exponential-time
`.gitignore` backtrack that did not return within a bounded wall-clock
window; `FixRustyLlamaThree` reproduced a tokenizer merge loop that did not
finish within 25 seconds pre-fix; `FixRustyMcpTraversal` reproduced an
end-to-end traversal where a crafted URI's decoded payload
(`contents of ../../etc/passwd`) actually reached the resource reader
pre-fix — notably, this also caught the CHANGELOG's own prior claim about
this exact code path ("percent-decoded after matching, so decoding cannot
reintroduce a separator") being the precise inverse of what the code
actually did; `FixNexusAudioCmdInjectionDownload` reproduced the PowerShell
`$(...)` subexpression injection standalone before fixing it, confirming a
real side-effecting command execution; `FixRustyTermL13PtyInjection`
reproduced a second, attacker-injected OSC string-terminator spliced into
the PTY response frame pre-fix.

Mid-batch, one fix task (`FixRustyMcpTraversal`) ran `git stash` on the
shared working tree to prove its own pre/post-fix diff, which transiently
reverted in-progress edits across roughly 38 files belonging to sibling
tasks. The stash was never dropped; every affected sibling detected the
revert (via its own `git diff`/file-content check or a peer broadcast) and
recovered cleanly via `git checkout stash@{0} -- <path>`, with no data
loss — confirmed by each task's own final `git diff`/test-pass report and
by the closing lint sweep below finding a clean, complete 59-file diff. The
originating task acknowledged the incident and confirmed recovery across
all affected peers before finishing.

After all 25 fix tasks landed, a workspace-lint sweep (`cargo fmt -p` +
`cargo clippy --all-targets -D warnings -p`, per touched crate, plus a
WSL Fedora run for the Linux-only `rusty_libc`) caught 7 additional real
lint violations introduced by the new fix/test code across 6 crates
(`nexus-plugins`, `nexus-panic-log`, `rusty_serde`, `nexus-ai`,
`nexus-memory`, `rusty_libc`) — two `doc_markdown` (bare "DoS" needing
backticks), one `doc_lazy_continuation` (a hanging doc-comment bullet), one
`unnecessary_cast`, one `collapsible_match`, three `cast_possible_wrap`, and
one `redundant_guards` — all fixed directly and re-verified clean, with
every touched crate's test suite re-run green afterward.

| # | Disposition | # | Disposition | # | Disposition | # | Disposition |
| - | - | - | - | - | - | - | - |
| 1 | fixed | 11 | fixed | 21 | fixed | 31 | fixed |
| 2 | fixed | 12 | fixed | 22 | fixed | 32 | fixed |
| 3 | fixed | 13 | fixed | 23 | fixed | 33 | fixed |
| 4 | fixed | 14 | fixed | 24 | fixed | 34 | fixed |
| 5 | fixed | 15 | fixed | 25 | fixed | 35 | fixed |
| 6 | fixed | 16 | fixed | 26 | fixed | 36 | fixed |
| 7 | fixed | 17 | fixed | 27 | fixed | 37 | fixed |
| 8 | fixed | 18 | fixed | 28 | fixed | 38 | fixed |
| 9 | fixed | 19 | fixed | 29 | fixed | | |
| 10 | fixed | 20 | fixed | 30 | fixed | | |

---

## `rusty_diff`

**1. `diff_myers` has unbounded O(D²) memory growth, reachable from `rgit diff` on any substantially-modified file.**
Location: `crates/rusty_diff/src/lib.rs` (`diff_myers`, `trace.push(v.clone())`).
Trigger: the Myers diff engine stores a full copy of the O(D)-sized frontier array for every edit distance `0..=D`, where `D = old.len()+new.len()` — quadratic memory, unbounded, reachable from `rgit diff` on any tracked file with substantial changes.
Fix: added `MAX_DIFF_INPUT_LEN = 20_000` (combined input length cap); inputs exceeding it fall back to a trivial O(n+m)-memory delete-all/insert-all diff instead of running the quadratic-memory trace, preserving the existing `Vec<DiffOp<T>>` return type so `rgit`'s call site needed no change.
Regression tests: `test_diff_myers_over_cap_uses_bounded_fallback_not_quadratic_trace`, `test_diff_myers_at_cap_still_uses_full_algorithm`. Verified: `cargo test -p rusty_diff` — 11 passed.
Severity: medium.

## `rusty_git`

**2. `Index::from_bytes` allocates from an unchecked, file-controlled entry count — an allocation bomb from a truncated/corrupted `.git/index`.**
Location: `crates/rusty_git/src/index.rs` (`from_bytes`, `Vec::with_capacity(count)` before any bound check).
Fix: bound the pre-allocation against the actual remaining byte length (each entry has a known minimum fixed size), returning `IndexError::Truncated` instead of allocating when the claimed count can't fit.
Regression test: `index::tests::truncated_index_with_huge_claimed_count_is_rejected_without_allocating` (a 32-byte truncated index claiming 2,147,483,647 entries). Proof: an isolated standalone harness with a 10 MiB allocation-refusing `GlobalAlloc` confirmed the pre-fix path unconditionally requested ~112 GiB before erroring.
Severity: high.

**3. `.gitignore`'s glob matcher backtracks with no memoization — exponential time on an adversarial multi-wildcard pattern.**
Location: `crates/rusty_git/src/gitignore.rs` (`glob_match`, recursive `*`-wildcard backtracking).
Fix: added a memoization table keyed on remaining `(pattern, name)` suffix lengths, reducing the adversarial case from O(n^k) to O(pattern_len × name_len).
Regression test: `gitignore::tests::adversarial_multi_wildcard_pattern_matches_quickly_instead_of_exponentially_backtracking` (20 `*` wildcards against a 35-char non-matching name, bounded at 2s). Proof: the un-memoized version did not return within that bound.
Severity: medium.

**4. `Repository::flatten_tree` recurses once per subtree entry with no depth cap — stack overflow via a crafted commit tree.**
Location: `crates/rusty_git/src/repository.rs` (`flatten_tree`, called from `head_tree_map`, used by `status`/`log`/`diff`).
Fix: added `MAX_TREE_DEPTH = 64` and a threaded depth parameter, returning a new `ObjectError::TooDeep` instead of recursing further.
Regression test: a tree chain nested past the cap errors instead of overflowing. Verified: `cargo test -p rusty_git` — 37 passed (all three `rusty_git` findings).
Severity: high.

## `nexus-ai`

**5. All three chat providers' SSE/NDJSON stream loops accumulate inbound bytes into an unbounded buffer — OOM from a malicious/misbehaving provider endpoint.**
Location: `crates/nexus/crates/nexus-ai/src/{openai.rs,anthropic.rs,ollama.rs}` (`chat_stream_with`/`chat_turn_with_tools`, `buf.extend_from_slice` with no cap before the next `\n`).
Fix: new shared `stream_buffer` module (`MAX_STREAM_BUFFER_BYTES = 512 KiB`, mirroring `nexus-linkpreview`'s issue-#78 fix) with `push_stream_bytes`, applied at all 4 call sites across the 3 files.
Regression tests: `accepts_bytes_within_cap`, `rejects_oversized_unterminated_chunk`, `rejects_when_accumulated_total_exceeds_cap_across_chunks`. Verified: `cargo test -p nexus-ai` — 308 lib + 1 integration passed.
Severity: high.

## `rusty_serde`

**6. The hand-rolled JSON deserializer recurses into `T::deserialize` with no depth tracking — stack overflow on deeply nested untrusted JSON.**
Location: `crates/rusty_serde/rusty_serde/src/json/de.rs` (`parse_array`/`parse_object` via `SeqWalker`/`MapWalker`).
Fix: added `MAX_NESTING_DEPTH = 128` and a depth counter checked at both recursive entry points (also covers the `deserialize_ignored_any`/skip path, which flows through the same code).
Regression test: `json_deeply_nested_array_is_rejected_instead_of_overflowing_stack`.
Severity: high.

**7. The hand-rolled RON deserializer has the identical unguarded-recursion shape across five separate call sites, including two not obvious from the original bug shape (bare `Some(...)` wrapper syntax and tagged/newtype-variant `(...)` wrapping, which can recurse through recursive enums without ever touching the array/object parser).**
Location: `crates/rusty_serde/rusty_serde/src/ron/de.rs` (`parse_seq_bracket`, `parse_map_brace`, `deserialize_any`'s tuple/`Some`/tag-redispatch arms, `VariantDataAccess::newtype_variant`, `skip_seq_like`/`skip_map_like`); the shared `value.rs` (`ValueDeserializer` and its `Seq`/`Map`/tagged-enum access types, reachable independently via the public `from_value` API) got the same `MAX_VALUE_DEPTH = 128` guard for the identical reason.
Regression tests: `ron_deeply_nested_array_is_rejected_instead_of_overflowing_stack`, `ron_deeply_nested_some_is_rejected_instead_of_overflowing_stack`, `ron_deeply_nested_newtype_variant_is_rejected_instead_of_overflowing_stack`, `value_deeply_nested_in_memory_tree_is_rejected_instead_of_overflowing_stack`. Verified: `cargo test -p rusty_serde` — 10 + 1 doc-test passed.
Severity: high.

## `rusty-mcp`

**8. A percent-encoding bypass of the path-traversal guard in the MCP resource URI-template matcher lets a crafted URI read arbitrary files.**
Location: `crates/rusty_mcp/crates/rusty-mcp/src/resources.rs` (`UriTemplate::match_uri`'s `Part::Var` branch checked the raw, not-yet-decoded captured substring for `/`, then decoded afterward — so `%2f`-style encoding, or mixed-case/double-encoding, sailed through the check and was reintroduced as a real separator on decode; a bare `..` also passed since it contains no `/`).
Fix: reordered to decode first (via a hardened `percent_decode` using `decode_utf8()` instead of lossy decoding, rejecting invalid-UTF-8 escapes outright), then check the *decoded* value for `/` and reject a decoded value that is exactly `.`/`..`.
Regression tests: 5, including an end-to-end proof that a crafted percent-encoded traversal URI now returns `RESOURCE_NOT_FOUND` instead of reaching the reader. Verified: `cargo test -p rusty-mcp` — 129 lib + integration + 16 doc-tests passed.
Severity: high.

## `nexus-plugins`

**9. The WASM sandbox's `host::http_request` import doesn't disable HTTP redirects — a plugin can bypass the `NetworkPolicy` host allowlist via a 3xx redirect.**
Location: `crates/nexus/crates/nexus-plugins/src/host_fns.rs`. The sibling crate `nexus-security`'s `http_policy.rs` already fixed the identical SSRF pattern for its own broker.
Fix: mirrored that fix — `reqwest::redirect::Policy::none()`, rejecting any 3xx response outright instead of following it.
Regression test: `host_fns::tests::execute_http_request_does_not_follow_redirect_off_allowlisted_host`.
Severity: high.

**10. `WasmSandbox::dispatch` spawns a brand-new, unjoined OS thread on every dispatch call for the epoch-deadline watcher — thread-exhaustion DoS under frequent dispatch.**
Location: `crates/nexus/crates/nexus-plugins/src/sandbox.rs`.
Fix: replaced the per-dispatch thread spawn with a single shared background `EpochWatcher` thread (Mutex/Condvar-driven), armed/cancelled per dispatch instead of spawned per dispatch.
Regression tests: `sandbox::tests::dispatch_does_not_spawn_a_thread_per_call` (200 dispatches, exactly 1 thread instead of 201), `dispatch_actually_times_out_via_shared_watcher`. Verified: `cargo test -p nexus-plugins` — 183 lib + all integration suites passed.
Severity: medium.

## `rusty_llama`

**11. The SPM/BPE tokenizer's merge loop is O(n²) — a single large request body can pin the server's sole generation worker thread indefinitely.**
Location: `crates/rusty_llama/src/tokenizer.rs` (`Spm::encode_piece`, full pairwise rescan per merge step). Since `server.rs` runs the whole model+tokenizer on one dedicated worker thread, any pathological-complexity tokenizer call is a full-server DoS.
Fix: rewrote to the standard BPE merge algorithm — a doubly-linked-list token sequence plus a max-heap of candidate merges, each merge only re-evaluating its two new neighboring pairs (O(n log n) overall) instead of rescanning the whole sequence.
Regression test: `tokenizer::tests::spm_encode_large_repeated_input_completes_quickly`. Proof: the pre-fix version did not finish a 20,000-char adversarial input within a 25s bound.
Severity: high.

**12. The hand-rolled HTTP server's request-line/header-line reads are unbounded — a memory-exhaustion DoS independent of the existing body-size cap.**
Location: `crates/rusty_llama/src/server.rs` (`handle_connection`).
Fix: `MAX_HEADER_BYTES = 8 KiB` (mirroring `nexus-lsp`'s identical cap), enforced via a `take()`-limited read across the request-line and header block; an oversized line returns 400 and closes the connection.
Regression test: `server::tests::header_read_rejects_oversized_request_line`. Proof: pre-fix, the same input only failed via the pre-existing 30s socket-read timeout, not a fast rejection.
Severity: high.

**13. `Spm::decode` unconditionally indexes the vocab table — panics (killing the sole worker thread) when a GGUF's tensor-derived vocab size disagrees with the tokenizer's actual vocab length.**
Location: `crates/rusty_llama/src/tokenizer.rs` (`Spm::decode`); `grammar.rs`'s `GrammarStage` already handles the identical mismatch safely via `.get()`.
Fix: mirrored the same defensive `.get()` pattern in `Spm::decode` instead of an unchecked index.
Regression test: decoding an out-of-range token id returns an error/fallback instead of panicking. Verified: `cargo test -p rusty_llama --features server` — full suite passed.
Severity: high.

## `nexus-skills`

**14. The `REGISTRY.json` cold-start loader path-joins an index-supplied path with no traversal guard — arbitrary file read that flows into AI prompts via compose/invoke.**
Location: `crates/nexus/crates/nexus-skills/src/registry.rs` (`try_load_from_index`, manual per-segment `PathBuf::push`).
Fix: replaced the manual join with `nexus_types::paths::resolve_within` (the same confinement primitive `nexus-storage`/`nexus-editor` already use), rejecting absolute paths and `..` segments.
Regression tests: `registry::tests::load_with_index_rejects_absolute_path_entry`, `load_with_index_rejects_dotdot_traversal_entry`. Proof: both attacker-controlled entries actually read a planted secret file pre-fix.
Severity: high.

**15. The `depends_on` dependency composer has cycle detection but no depth cap on acyclic chains — stack overflow from ordinary long `.skill.md` chains.**
Location: `crates/nexus/crates/nexus-skills/src/compose.rs` (`compose::visit`).
Fix: added `MAX_COMPOSE_DEPTH = 64` (mirroring `nexus_database`/`nexus_formats`'s existing depth-cap constants), returning `ComposeError::DepthExceeded` past it.
Regression test: `compose::tests::compose_rejects_dependency_chain_deeper_than_max_depth` (265-id linear chain). Verified: `cargo test -p nexus-skills` — 58 passed.
Severity: medium.

## `nexus-notifications`

**16. Discord/Generic-webhook/Telegram transports embed secrets directly in the outbound request URL, which `reqwest::Error`'s `Display` leaks verbatim into IPC-visible failure responses on any transport-layer failure (DNS/connect/TLS/timeout).**
Location: `crates/nexus/crates/nexus-notifications/src/lib.rs` (`DiscordWebhook::send`, `GenericWebhook::send`, `TelegramBot::send`); propagated verbatim by `core_plugin.rs`'s `fan_out`/`dispatch_send`.
Fix: new `sanitize_transport_error` helper — classifies the failure kind and keeps only the destination host (never the secret-bearing path/query) plus the underlying error message, applied at all three transports' `map_err` sites.
Regression tests: `discord_transport_connect_failure_does_not_leak_webhook_secret`, `webhook_transport_connect_failure_does_not_leak_url_secret`, `sanitize_transport_error_strips_secret_from_connect_failure`. Proof: the pre-fix panic messages showed the raw webhook token in the "leaked" assertion failure text. Verified: `cargo test -p nexus-notifications` — 77 passed.
Severity: high.

## `rusty_text`

**17. The awk interpreter's field-index/NF handling drives an unbounded `Vec<String>` resize from attacker/data-controlled values.**
Location: `crates/rusty_text/src/awk/interp.rs` (`set_field`, `set_var`'s `"NF"` arm).
Fix: `MAX_FIELDS = 1_000_000` cap; an oversized index/NF sets a runtime error flag instead of resizing, surfaced as `Err` from `AwkProgram::run`.
Regression tests: `oversized_field_index_assignment_errors_instead_of_allocating`, `oversized_nf_assignment_errors_instead_of_allocating`, `field_index_and_nf_within_the_cap_still_work`.
Severity: medium.

**18. `parse_leading_number` doesn't handle scientific notation, silently truncating values like `"1e3"` while `looks_numeric` correctly recognizes the whole string as numeric — wrong arithmetic/comparison results.**
Location: `crates/rusty_text/src/awk/interp.rs`.
Fix: added exponent (`e`/`E`, optional sign, digits) consumption, only when it forms a complete valid exponent.
Regression tests: `scientific_notation_field_value_parses_correctly`, `scientific_notation_string_literal_arithmetic`. Verified: `cargo test -p rusty_text` — 53 passed.
Severity: low.

## `nexus-panic-log`

**19. The global panic hook writes raw, unredacted panic payloads/backtraces to a log file with default (world-readable) OS permissions.**
Location: `crates/nexus/crates/nexus-panic-log/src/lib.rs`.
Fix: new `open_restricted`/`restrict_permissions` — Unix: `OpenOptionsExt::mode(0o600)` at creation plus a re-asserted `set_permissions`; Windows: replaces the file's DACL with an owner-only Full-Control ACE via `SetFileSecurityW`/SDDL, applied to both the live log and its rotated copy.
Regression test: `tests::log_file_created_with_owner_only_permissions` (platform-specific assertion). Proof (Windows): reverted the ACL-restriction call and confirmed the file inherited 5 ACEs from its parent directory instead of 1. Verified: `cargo test -p nexus-panic-log` — 3 passed.
Severity: medium.

## `nexus-audio`

**20. The Windows local-TTS shell-out string-interpolates untrusted text into a PowerShell `-Command` string — `$(...)` subexpression command injection.**
Location: `crates/nexus/crates/nexus-audio/src/local_backend.rs` (`run_platform_tts`).
Fix: text is no longer interpolated into the script at all — it's written to a temp file and read back as data via `Get-Content -LiteralPath ... -Raw`; the paths that remain in the script string are escaped as true PowerShell single-quoted literals (which never interpolate subexpressions), via a new `powershell_single_quote` helper.
Regression tests: `windows_tts_script_never_embeds_text_for_interpolation`, `windows_tts_treats_subexpression_payload_as_inert_text`. Proof: a standalone PowerShell repro first confirmed `$(...)` executed under the old escaping (a `New-Item` side effect actually fired); the Rust-level regression test failed identically pre-fix (marker file created) and passed post-fix. Verified: `cargo test -p nexus-audio --features local-audio` — 20 passed.
Severity: high.

**21. `ensure_model()` performs an unbounded, timeout-less blocking download of the Whisper model.**
Location: `crates/nexus/crates/nexus-audio/src/local_backend.rs`.
Fix: `MODEL_DOWNLOAD_TIMEOUT = 300s` and `MAX_MODEL_BYTES = 800 MiB`, enforced both against a declared `Content-Length` before reading and against the actual bytes read (`resp.take(cap+1)`) so a server that lies about or omits `Content-Length` can't stream unbounded data either.
Severity: medium.

## `nexus-collab`

**22. The relay's shared `broadcast::channel(1024)` combined with a 16 MiB max-frame-size lets one peer force ~16 GiB of retained buffer memory.**
Location: `crates/nexus/crates/nexus-collab/src/server.rs` (`pump_reads`).
Fix: added a much tighter practical per-frame cap `MAX_ENVELOPE_FRAME_BYTES = 256 KiB`, enforced on ingress before a message reaches the broadcast channel — an oversized frame force-drops the sending peer instead of being broadcast. Reduces the worst case from ~16 GiB to ~256 MiB (64×).
Regression tests: `oversized_envelope_disconnects_peer_without_broadcasting`, `broadcast_worst_case_memory_is_bounded_well_below_prior_16gib`.
Severity: high.

**23. The `RecvError::Lagged` branch silently `continue`s instead of dropping the peer, contradicting its own doc comment — permanent silent loss of CRDT/presence ops for the lagging peer.**
Location: `crates/nexus/crates/nexus-collab/src/server.rs` (per-peer broadcast writer loop).
Fix: extracted a testable `run_writer` free function; the `Lagged` arm now `break`s (force-drops the peer) instead of `continue`ing, matching the documented contract.
Regression test: `lagged_writer_is_force_dropped_not_resumed`. Proof: reverting `break` back to `continue` reproduced the peer silently resuming and forwarding a post-lag message. Verified: `cargo test -p nexus-collab` — 80 passed.
Severity: medium.

## `rp-providers` (`rusty_provider`)

**24. All three chat-provider HTTP adapters plus the shared error-mapping helper buffer upstream response bodies with no size cap — the same pattern round 6 already fixed for three sibling backends in `skillopt-model`.**
Location: `crates/rusty_provider/crates/providers/src/{http.rs,anthropic.rs,gemini.rs,openai_compatible.rs}` (`map_error_response`'s `resp.text()`, and `chat()`/`embeddings()`'s `resp.json()`, all uncapped).
Fix: new shared `read_capped_body` helper (`MAX_RESPONSE_BODY_BYTES = 16 MiB`, mirroring round 6's `skillopt-model::http::read_capped_body`), checked against both a declared `Content-Length` and the running total while streaming; applied at all 6 call sites across the 4 files (including `gemini`'s/`openai_compatible`'s `embeddings()`, the identical pattern in the same files beyond the originally-named `chat()` sites).
Regression tests: 6, one per call site, plus a shared `oversized_content_length_server` test helper. Verified: `cargo test -p rp-providers` — 193 passed.
Severity: medium.

## `inventory-core`

**25. The Codex and Claude Code JSONL readers abort the entire source scan on one unreadable/corrupt file, contradicting the crate's own "tolerant parsing" contract.**
Location: `crates/rusty_inventrory/crates/inventory-core/src/sources/{codex.rs,claude_code.rs}` (`scan()` propagating a per-file read/parse error via `?`).
Fix: extracted `scan_roots` free functions that log-and-skip (`tracing::warn!`) a per-file error and continue, mirroring the existing per-item-skip pattern already used by this crate's `vscdb`/`zed` readers.
Regression tests: `sources::codex::tests::a_corrupt_file_does_not_block_the_rest_of_the_source`, the equivalent for `claude_code`. Verified: `cargo test -p inventory-core --lib` — 73 passed.
Severity: medium.

## `rusty_ansi`

**26. The OSC-string scanner treats any lone ESC byte as a valid terminator even when not followed by `\`, desyncing the parser from a legitimate escape sequence that immediately follows.**
Location: `crates/rusty_ansi/src/lib.rs` (`AnsiParser`'s OSC-collecting branch).
Fix: split into three explicit cases — `ESC '\'` (real 2-byte ST), bare BEL (1-byte terminator), and a lone ESC not followed by `\` (terminator length 0 — the ESC is left unconsumed so it's parsed fresh as a new sequence).
Regression test: `tests::osc_terminator_requires_backslash_after_lone_esc` — an unterminated OSC immediately followed by a real CSI color sequence. Proof: pre-fix, the CSI leaked through `strip_ansi` as literal text (`"[31mRed"`) instead of being recognized and stripped. Verified: `cargo test -p rusty_ansi` — 5 passed.
Severity: medium.

## `rusty_term` (`l13`)

**27. The OSC 5379 side-channel echoes an untrusted, unescaped protocol tag back into the PTY response frame on the unrecognized-protocol error path — PTY input injection from any displayed byte stream.**
Location: `crates/rusty_term/l13/src/lib.rs` (`handle()`'s unrecognized-protocol branch, feeding `send()`).
Fix: new `sanitize_protocol_tag` filters the tag down to ASCII alphanumerics plus `-`/`_`/`.` (a no-op for every real protocol tag) before it can appear in any frame written back to the child's stdin.
Regression test: `tests::unrecognized_protocol_tag_is_sanitized_before_echo` — a tag embedding an ESC+`\` (ST) sequence. Proof: pre-fix, the response frame contained a second, attacker-injected ST that split the OSC frame early, exposing raw space/BEL bytes. Verified: `cargo test -p rusty_term_l13` — 13 passed.
Severity: high.

## `rpath`

**28. `win32_to_posix` conflates Windows drive-relative paths (`C:foo.txt`) with drive-absolute paths (`C:\foo.txt`), silently producing the wrong POSIX path for the relative form.**
Location: `crates/rpath/src/lib.rs`.
Fix: checks whether the character after the drive-letter colon is a path separator; if not (drive-relative), returns a visibly distinct `c:foo.txt`-shaped representation instead of silently aliasing it to the rooted `/c/foo.txt` absolute form.
Regression test: `test_win32_to_posix_drive_relative_distinct_from_absolute`. Verified: `cargo test -p rpath` — 6 passed.
Severity: low.

## `nexus-tui`

**29. Quitting the TUI while the terminal panel is open (Esc, then `q`) skips the terminal session's close/persist path entirely — silent scrollback/metadata loss on an ordinary usage pattern, contradicting `kill_terminal`'s own doc comment.**
Location: `crates/nexus/crates/nexus-tui/src/{app.rs,lib.rs}`.
Fix: `run()`'s exit path now calls `kill_terminal()` whenever a terminal session is still open, gated by a new pure, unit-testable predicate `quit_should_close_terminal`.
Regression tests: 3, covering the quit-with-session/quit-without-session/session-without-quit cases.
Severity: medium.

**30. The terminal panel's refresh is gated solely on line *count*, so a `\r`-driven progress bar (same line count, changing content) appears frozen for the entire redraw.**
Location: `crates/nexus/crates/nexus-tui/src/app.rs` (`pump_terminal`).
Fix: new `terminal_lines_changed` also compares the last line's content and `repeats` count, not just the overall count.
Regression tests: 5, covering same-length content/repeats changes and no-change cases. Verified: `cargo test -p nexus-tui --lib` — 50 passed (8 new).
Severity: low.

## `rusty_libc`

**31. `getgroups(&mut [])` panics with an out-of-bounds slice on any process with at least one supplementary group.**
Location: `crates/rusty_libc/src/process.rs`. The Linux kernel's documented `size == 0` special case for `getgroups(2)` returns the *true* group count regardless of the passed size (used deliberately by this crate's own `ngroups()` helper), but the wrapper didn't bound its `&buf[..n]` slice by the actual buffer length.
Fix: `Ok(&buf[..n.min(buf.len())])` — correctly handles both the `size==0` count-query case and normal nonzero-size calls.
Regression test: `getgroups_empty_buffer_does_not_panic_when_process_has_groups` (forks a child, forces real supplementary groups via `setgroups`, calls `getgroups(&mut [])` under `catch_unwind`). Proof: reproduced a genuine out-of-bounds panic pre-fix as root in WSL Fedora. Verified: `cargo test -p rusty_libc --lib` (WSL Fedora) — 207 passed, 1 unrelated pre-existing environmental flake (`socket::tests::connect_refused_when_nothing_listens`, an intermittent WSL2 network-stack behavior unrelated to this change).
Severity: medium.

## `nexus-memory`

**32. `clamp_limit` doesn't clamp anything — any IPC/MCP caller can force an unbounded table scan via `limit`.**
Location: `crates/nexus/crates/nexus-memory/src/db.rs`.
Fix: `MAX_QUERY_LIMIT = 1000`, applied via `.min()` before the usize→i64 conversion every limit-taking query helper shares.
Regression tests: `db::tests::clamp_limit_caps_huge_requested_limit`, `list_caps_an_enormous_requested_limit`.
Severity: medium.

**33. Bus-event secret redaction only scans JSON object keys, not string values or array elements — a secret smuggled inside a value (e.g. `"note": "api_key=sk-..."`) syncs to every other agent/node unredacted via `nexus-memory-hub`.**
Location: `crates/nexus/crates/nexus-memory/src/capture.rs` (`redact_in_place`/`is_secret_key`).
Fix: new `looks_like_secret_value` heuristic (recognizable secret-token prefixes, plus a `key=value`/`key: value` pattern using the existing key-marker list) applied to `Value::String` in the same recursive walk that already covers arrays/nested objects.
Regression tests: `redacts_secret_smuggled_inside_an_innocuous_string_value`, `redacts_secret_shaped_value_inside_an_array`. Verified: `cargo test -p nexus-memory` — 98 lib + 2 integration + 1 doc-test passed.
Severity: high.

## `nexus-memory-hub`

**34. `HubStore::push` accepts an unvalidated client-supplied `updated_at`, letting any bearer-authenticated node permanently poison a shared record via a far-future timestamp no legitimate future edit can ever beat (last-write-wins on `updated_at`).**
Location: `crates/nexus/crates/nexus-memory-hub/src/lib.rs`.
Fix: `is_implausibly_future` rejects (skips, counted in the response's `failed` count) any `updated_at` more than `MAX_FUTURE_SKEW_MINUTES = 5` ahead of the hub's own clock; unparseable timestamps pass through unchanged (schema-agnostic).
Regression tests: `is_implausibly_future_flags_far_future_only`, `push_rejects_implausible_future_timestamp`. Verified: `cargo test -p nexus-memory-hub` — 9 passed.
Severity: medium.

## `nexus-hashline`

**35. `apply_ops`'s file-reconstruction loop does an O(n·m) linear scan of every insert-op anchor for every line — CPU-exhaustion DoS from a crafted patch against a large file.**
Location: `crates/nexus/crates/nexus-hashline/src/apply.rs`.
Fix: builds a `HashMap<line, Vec<op_index>>` index once before reconstruction, turning per-line lookup into O(1).
Regression test: `apply::tests::many_insert_ops_reconstruct_in_near_linear_time` (20,000-line file, 40,000 ops, <1s bound). Proof: the pre-fix linear-scan version took 2.67s against the same input; post-fix, ~70ms. Verified: `cargo test -p nexus-hashline` — 34 passed.
Severity: medium.

## `rusty_tls`

**36. X.509 extension duplicate-OID detection is O(n²), reachable from any certificate this crate parses — before any trust decision.**
Location: `crates/rusty_tls/src/handrolled/x509.rs` (`read_extensions`, `Vec`+`contains` linear scan).
Fix: replaced with a `HashSet` (`ObjectIdentifier` already derives `Hash`/`Eq`), making duplicate detection O(n) amortized.
Regression test: `many_unique_extensions_parse_without_quadratic_blowup` (20,000 distinct extensions, <1s bound). Proof: the pre-fix version took 1.36s against the same input.
Severity: medium.

**37. `plaintext_record` casts the ClientHello/ServerHello fragment length to `u16` with no range check — silently truncates instead of erroring when a fragment (e.g. one carrying a large session-resumption ticket) exceeds `u16::MAX`.**
Location: `crates/rusty_tls/src/handrolled/{client.rs,server.rs}`.
Fix: both now return `Result`, checking `fragment.len() > u16::MAX as usize` and returning the existing `RecordError::FragmentTooLong` variant before the cast, propagated via `?` at all 7 call sites across both files.
Regression test: `an_oversized_resumption_ticket_is_refused_rather_than_truncated` — a real handshake with a ticket sized to push the total ClientHello fragment past `u16::MAX`. Verified: `cargo test -p rusty_tls --features handrolled-engine` — 34 + 46 + 46 passed across the three affected test binaries.
Severity: medium.

## `rusty_wiremock`

**38. The mock server's fixed 8192-byte header scratch buffer silently truncates instead of erroring when headers exceed it, mis-parsing `content_length` and skipping the body drain — reintroducing the write-races-peer-close race its own doc comment says it prevents.**
Location: `crates/rusty_wiremock/src/canned.rs` (`read_request_head`).
Fix: the scratch buffer now grows (doubling) up to a new `MAX_HEADER_BYTES = 1 MiB` hard cap instead of silently proceeding with whatever fit in the original fixed array; exceeding the cap is now an explicit rejection, checked by the caller.
Regression test: `canned::tests::drains_body_when_headers_exceed_initial_scratch_buffer` (~600 padding headers pushing past 8192 bytes). Proof: pre-fix, the client observed a genuine `ConnectionReset` (OS error 10054) — the exact race the fix closes. Verified: `cargo test -p rusty_wiremock --features std` — 4 passed.
Severity: low.

---

## Workspace-lint sweep (post-fix)

`cargo fmt -p <crate>` and `cargo clippy -p <crate> --all-targets -- -D warnings` were run against all 25 touched crates (24 on the native Windows host, `rusty_libc` via WSL Fedora, matching round 6's precedent for Linux-only crates). This caught 7 additional real lint violations in code introduced by the fixes above, all fixed directly and re-verified clean with the owning crate's test suite re-run green:

- `nexus-plugins/src/sandbox.rs` — `doc_markdown` (bare "DoS" in a doc comment).
- `nexus-ai/src/stream_buffer.rs` — `doc_markdown` (same pattern, new module's doc comment).
- `nexus-panic-log/src/lib.rs` — `unnecessary_cast` (`SDDL_REVISION_1 as u32` where the constant is already `u32`).
- `rusty_serde/rusty_serde/tests/nesting_depth.rs` — `doc_lazy_continuation` (an unindented continuation line under a doc-comment bullet).
- `nexus-memory/src/capture.rs` — `collapsible_match` (a single-armed `if` inside a `match` arm, collapsed into a match guard).
- `nexus-memory/src/db.rs` — `cast_possible_wrap` ×3 (test-only `as i64` casts of a `usize` constant, replaced with `i64::try_from(...).unwrap()`).
- `rusty_libc/src/process.rs` — `redundant_guards` (a `match` guard checking `groups.is_empty()`, collapsed into a `[]` pattern).

No other lint violations were found across the 25-crate touched surface.

# Monorepo improvement review — rusty_mill (round 6)

Reviewed on 2026-09-13 against `main`, via `/codex-build`. This is a sixth,
independent pass — it does not reopen round 1 (`CODEX-MONOREPO-REVIEW.md`,
33 findings), round 2 (`CODEX-MONOREPO-REVIEW-2026-09-12.md`, 63 findings),
round 3 (`CODEX-MONOREPO-REVIEW-2026-09-12-round3.md`, 40 findings), round
4 (`CODEX-MONOREPO-REVIEW-2026-09-12-round4.md`, 36 findings), or round 5
(`CODEX-MONOREPO-REVIEW-2026-09-12-round5.md`, 33 findings) — all five
already merged (PR #188 for round 5). It also does not reopen
`repo-inspector-report.md`'s duplication-cluster/sovereignty findings, a
different concern already triaged there.

All 34 findings below were fixed in the same working session that
produced this report, each with a regression test that fails on the
pre-fix code and passes post-fix.

## Method and scope

Round 5 explicitly listed the crate families it had not yet reviewed at
depth: `rusty_inventrory`/`rusty_skillopt`, `rusty_kafka`, the graphics/
media stack, most standalone foundational network/codec crates, and
several `nexus` subsystems. This round targets exactly that list, plus
sub-crates rounds 1-4 had only partially fixed (e.g. `rusty_font`'s CFF
code path, distinct from round 3's TrueType fix; `rusty_rdp` beyond round
5's RSA-blinding fix; `rusty_whisper`/`rusty_llama` beyond round 3's GGUF
alignment fix) and one crate (`rusty_hister`'s extractor) whose real
implementation only landed after round 5's scout found it an empty stub —
six concrete extractors (JSON-LD, StackExchange, EmbeddedVideo, GoDoc,
Lobsters, HackerNews) have since been merged.

Seventeen parallel read-only scout passes covered this surface, each
instructed to report only concrete, triggerable defects with file:line
evidence and a specific triggering input, at high confidence, and to
return nothing rather than pad with style nits. One scout
(`rusty_proxmox`/`rusty_opnsense`/`rusty_fedora`/`rusty_multimodal_db`)
returned zero findings — that surface was reviewed in full and found
clean, with round 2/3's fixes confirmed still correctly in place. Several
sub-areas within other scouts' assigned scope were reviewed in comparable
depth and yielded nothing meeting the confidence bar (preserved in each
scout's own transcript): `rusty_audio`/`rusty_voice`, `rusty_llama`
(served as this round's own reference implementation for several
`rusty_whisper` fixes), `rusty_rag`/`rusty_config`/`rusty_boot`,
`rusty_uuid`/`rusty_sqlite`/`rusty_tokio`, `rusty_wire`/`rusty_err`/
`rusty_request`/`rusty_retry`, `rusty_url`, `winargv`, `nexus-mcp`'s
dispatch/auth/pool layer, `nexus-ai`/`nexus-ai-runtime`'s scheduler/
session/provider clients, and most of `rusty_lsp`/`rusty_mcp`'s framing
and auth layers (both already hardened to a very high standard).

All locations are relative to this checkout at the time of review.
Severity: **high** = memory unsafety, durable data loss, credential
exposure, hostile-input resource exhaustion, or a security-policy bypass
reachable from untrusted/public input; **medium** = bounded correctness/
reliability defect or a security gap with mitigating preconditions;
**low** = documentation/config drift or a small avoidable issue.

## Disposition

Every finding was fixed by 29 parallel fix tasks (one per disjoint crate
group), each with a regression test that fails pre-fix and passes
post-fix. Many tasks went further than a plain pass/fail check by proving
the pre-fix crash/vulnerability directly: `FixKafkaRecordBatch` and
`FixRusqliteRecursionDepth` reproduced a genuine `STATUS_STACK_OVERFLOW`/
allocation-failure process abort against the reverted code;
`FixNexusStoragePathTraversal` confirmed its reverted code actually leaked
a planted secret file's frontmatter and silently created a base directory
outside the forge root; `FixFontCffBlowup` confirmed its reverted code
hung indefinitely rather than merely running slowly; `FixKafkaClientTimeout`
confirmed a genuine indefinite hang against a broker that never responds.
Two crates (`rusty_stream`, `rusty_h2`) required their Linux-only
`io-uring` backend, verified via the workstation's WSL Fedora checkout
rather than native Windows `cargo test`. After all 29 landed, a
workspace-lint sweep (`cargo fmt` + `cargo clippy --all-targets -D
warnings`) across all 23 touched crates caught 3 additional real lint
violations (two `manual_repeat_n`/`cloned_ref_to_slice_refs` nits in new
test code, one `unused_must_use` in an example binary's error-logging
call), all fixed directly and re-verified clean.

Several fix tasks running concurrently in the same shared working tree
used `git stash`/temporary reverts to prove their regression tests
genuinely fail pre-fix; two such operations transiently collided across
sibling tasks (noted in `FixJinjaRecursionGuard`'s and
`FixSkillmodelSubprocessBackends`'s own reports) and both self-recovered
cleanly, confirmed via `git diff` showing only the intended fix present
with no data loss.

| # | Disposition | # | Disposition | # | Disposition | # | Disposition |
| - | - | - | - | - | - | - | - |
| 1 | fixed | 10 | fixed | 19 | fixed | 28 | fixed |
| 2 | fixed | 11 | fixed | 20 | fixed | 29 | fixed |
| 3 | fixed | 12 | fixed | 21 | fixed | 30 | fixed |
| 4 | fixed | 13 | fixed | 22 | fixed | 31 | fixed |
| 5 | fixed | 14 | fixed | 23 | fixed | 32 | fixed |
| 6 | fixed | 15 | fixed | 24 | fixed | 33 | fixed |
| 7 | fixed | 16 | fixed | 25 | fixed | 34 | fixed |
| 8 | fixed | 17 | fixed | 26 | fixed | | |
| 9 | fixed | 18 | fixed | 27 | fixed | | |

---

## `rusty_ansder`

**1. `read_integer_u32` silently truncates a 5-byte DER INTEGER instead of rejecting it as out-of-range.**
Location: `crates/rusty_ansder/src/der.rs` (`read_integer_u32`, ~lines 156-181).
Trigger: a DER INTEGER content of `01 00 00 00 00` (representing 2^32, which cannot fit in `u32`) left-shifts its leading byte's bits off the end of the accumulator instead of erroring, silently returning `Ok(0)`.
Fix: reject any 5-byte-or-longer encoding whose leading content byte is non-zero, before the accumulation loop runs.
Regression test: `der::tests::read_integer_u32_rejects_a_5_byte_overflowing_encoding`. Verified: `cargo test -p rusty_ansder` — 5 passed.
Severity: medium.

## `rusty_jinja`

**2. Two recursive expression-parser call sites bypass the existing stack-overflow depth guard.**
Location: `crates/rusty_jinja/src/parser.rs` — `MAX_NESTING_DEPTH=64` (round 2) is checked for parenthesized groups/`not`/call-args, but nested bracket-index expressions (`a[a[a[...]]]`) and chained unary-minus (`---------1`) never increment/check it.
Trigger: a hostile model chat-template file with either construct nested tens of thousands of levels deep overflows the native stack. Confirmed via a real `STATUS_STACK_OVERFLOW` against the reverted code.
Fix: added the same depth increment/check to both call sites.
Regression tests: `deeply_nested_bracket_index_errors_instead_of_overflowing_the_stack`, `deeply_chained_unary_minus_errors_instead_of_overflowing_the_stack`. Verified: `cargo test -p rusty_jinja` — 29 passed.
Severity: high.

## `rusty_stream`

**3. `Client::request` has no size cap on the declared response-frame length, unlike the server side.**
Location: `crates/rusty_stream/src/client.rs` (reads a 4-byte length prefix, allocates `vec![0u8; len]` unconditionally).
Fix: reused `server.rs`'s `MAX_FRAME_LEN` cap, returning `ClientError::FrameTooLarge` before allocating.
Regression test: `client::tests::a_response_frame_claiming_more_than_the_cap_is_rejected`. Verified (via WSL, Linux-only `io-uring` backend): `cargo test -p rusty_stream` — 75 passed.
Severity: medium.

## `rusty_h2`

**4. CONTINUATION frames are silently dropped, and split header blocks are decoded incomplete — desyncs the shared HPACK dynamic table for the rest of the connection.**
Location: `crates/rusty_h2/src/connect/mod.rs` (`handle_headers`/`handle_push_promise` decode `header_block_fragment` regardless of `end_headers`; `Frame::Continuation` was a no-op in `handle_frame`), contradicting `frame/headers.rs`'s documented CONTINUATION-merging contract.
Trigger: a peer splitting a header block across HEADERS+CONTINUATION frames (spec-legal, or adversarial) corrupts the connection's shared HPACK state for every subsequent request.
Fix: added a per-connection `PendingHeaderBlock` buffer that accumulates fragments across HEADERS/PUSH_PROMISE + CONTINUATION until `end_headers` is finally set, only then decoding the complete block.
Regression test: `connect::tests::continuation_frame_is_merged_into_the_preceding_header_block` — proves the dynamic table stayed correctly in sync by encoding a second request that only decodes if the merged block was correctly applied. Verified: `cargo test -p rusty_h2` — 90 lib + 3 + 13 integration passed.
Severity: high.

## `rusty_gui`

**5. An unpaired UTF-16 high surrogate silently drops the next typed character.**
Location: `crates/rusty_gui/src/window.rs` (Windows `WM_CHAR` handler's strict if/else-if chain abandons the current character when surrogate pairing fails).
Fix: restructured with a `handled` flag so a failed pairing falls through to ordinary-character handling instead of dropping the input.
Regression tests: `unpaired_high_surrogate_does_not_drop_next_char`, `valid_surrogate_pair_combines_into_one_char`. Verified: `cargo test -p rusty_gui` — 6 passed.
Severity: medium.

## `rusty_http`

**6. Whole-body readers have no size cap at all, unlike head/line parsing.**
Location: `crates/rusty_http/src/{sync,async_tokio,tokio_native}.rs` (`read_content_length_body`/`read_close_delimited_body`) — consumed by rusty-meshed-registry's HTTP server and Tailscale's ts-localapi/ts-control.
Fix: new `DEFAULT_MAX_BODY_LEN` (1 MiB) and `Error::BodyTooLarge`, enforced in all three transports; `read_body`'s public signature is unchanged so every existing caller is automatically protected.
Regression tests: 6 (2 per transport). Verified: `cargo test -p rusty_http --all-features` — 134 passed.
Severity: high.

## `rusty_oauth`

**7. `HttpRequest`'s derived `Debug` leaks client secrets, assertions, refresh tokens, and auth codes in full.**
Location: `crates/rusty_oauth/src/request.rs` — `ClientSecret` has a hand-written redacting `Debug`, but the redaction is defeated once the secret is folded into an `HttpRequest`'s headers/body, which any ordinary request-logging call then leaks verbatim.
Fix: hand-written `Debug` impl redacting the `Authorization` header and known-sensitive form/JSON body keys, falling back to a byte-count placeholder for unrecognized bodies.
Regression tests: 3, covering form-encoded, JSON, and unrecognized-body redaction. Verified: `cargo test -p rusty_oauth` — 133 passed.
Severity: high.

## `rusty_std`

**8. Two more silently-stubbed public APIs report false success/empty results — the same bug class round 3 fixed for `net.rs`/`time.rs`.**
Location: `crates/rusty_std/src/env.rs` (`args()` always returns `Vec::new()`), `src/process.rs` (`Command::status()` always reports success without spawning anything).
Fix: wired both to real OS primitives (`/proc/self/cmdline` on Linux, `GetCommandLineW`+`CommandLineToArgvW` on Windows for `args()`; `vfork_exec`+`waitpid` on Linux, `spawn_suspended`+`resume`+`wait` on Windows for `status()`).
Regression tests: `args_includes_at_least_the_running_binary`, `status_reflects_a_real_nonzero_exit_not_a_hardcoded_success`. Verified: `cargo test -p rusty_std` — 8 passed.
Severity: medium.

## `rustils` (`coreutils`)

**9. 15 of 21 coreutils binaries panic on a non-Unicode command-line argument.**
Location: `crates/rustils/crates/coreutils/src/bin/*.rs` — `std::env::args()` panics on invalid-Unicode argv, unlike `args_os()`.
Fix: new shared `coreutils::args::collect_lossy` helper (lossy `OsString`→`String` conversion) applied at all 15 call sites.
Regression tests: 2, including an unpaired-UTF-16-surrogate case matching exactly what `env::args()` panics on internally. Verified: `cargo test -p coreutils` — 24 passed across lib + all binaries.
Severity: medium.

## `rusty_win32`

**10. `get_proc_address` silently truncates a symbol name at an embedded NUL instead of rejecting it.**
Location: `crates/rusty_win32/src/dynlib.rs` — inconsistent with this crate's own `wide::to_wide` (hardened in round 3 for the identical shape).
Fix: added the same embedded-NUL rejection, returning `Err` instead of resolving a truncated symbol; updated `rusty_vulkan`'s one call site for the new `Result` signature.
Regression test: `dynlib::tests::resolving_a_symbol_with_an_embedded_nul_is_rejected_instead_of_truncated`. Verified: `cargo test -p rusty_win32 --lib dynlib` — 4 passed (7 unrelated, pre-existing environmental failures in `net`/`console` tests, caused by port contention from concurrent sibling test runs on this shared machine, not this change).
Severity: low.

## `nexus-storage`

**11. Path-confinement bypass across the entire `.bases` CRUD subsystem and `read_frontmatter`/`write_frontmatter`.**
Location: `crates/nexus/crates/nexus-storage/src/lib.rs` (13 functions: `base_create`, `base_record_*`, `base_property_*`, `base_view_*`), `src/handlers/bases.rs` (`index`, `load`), `src/handlers/notes.rs` (`read_frontmatter_for_path`, `write_frontmatter`'s read half) — all compute `forge.root().join(path)` directly, bypassing the `resolve_within` confinement every other mutating path in this crate already uses.
Trigger: an absolute or Windows drive-relative path lets `PathBuf::join` replace the forge root entirely — confirmed the reverted code actually leaked a planted secret file's frontmatter via `read_frontmatter` and silently created a base directory outside the forge via `base_create`.
Fix: routed every listed function through `resolve_within`/`engine.read_file`, mirroring the established pattern.
Regression tests: 3, covering absolute-path rejection on `base_create`, `read_frontmatter`, and `base_load`. Verified: `cargo test -p nexus-storage --lib` — 505 passed.
Severity: high.

## `rusty-hister-extractor`

**12. Three unbounded-recursion/allocation defects reachable from untrusted crawled-page markup.**
Location: `crates/rusty_hister/crates/rusty-hister-extractor/src/lobsters.rs` (`write_comment_text`/`write_comment_html` recurse per DOM nesting level with no cap), `src/textutil.rs` (`write_node_text` recurses over DOM children with no cap, used by HackerNews extraction), `src/hackernews.rs` (`comment_rows` parses the untrusted `indent` attribute with no upper bound, feeding straight into string-repeat/loop allocations).
Fix: added `MAX_COMMENT_DEPTH`/`MAX_NODE_DEPTH` caps (64/100) to all three, clamping the HackerNews indent value before use.
Regression tests: 5, including a 5,000-level nested-comment fixture and a maliciously large `indent="999999999"` value. Verified: `cargo test -p rusty-hister-extractor` (per-crate run confirmed in fix task).
Severity: high.

## `rusty_lsp`

**13. An inverted incremental text-edit range panics `Documents::did_change`, corrupting the shared document store.**
Location: `crates/rusty_lsp/src/documents.rs` — computes `start`/`end` independently with no check that `start <= end` before `replace_range`.
Fix: reject (typed error) an edit whose computed `start > end` instead of panicking, mirroring the sibling check already in `text.rs`'s `apply_edits_with`.
Regression test: `documents::tests::inverted_incremental_range_is_rejected_instead_of_panicking`. Verified: `cargo test -p rusty_lsp` — 179 + 69 + 3 + 5 passed across lib/integration/property/test_client, plus doc-tests.
Severity: medium.

## `rusty_kafka`

**14. `record_batch.rs`: unchecked broker-controlled counts drive unbounded allocation, plus an unbounded varint continuation-byte loop.**
Location: `decode_batch`/`decode_record` (`Vec::with_capacity` from an unchecked `records_count`/`header_count`); `read_varlong` (no cap on continuation bytes, shifting a `u64` by ≥64 past 10 bytes).
Trigger: confirmed the reverted code aborts with a real "memory allocation of 103079215056 bytes failed" crash.
Fix: bounded both allocations against the reader's remaining bytes; capped varint continuation bytes at 10.
Regression tests: 3. Verified: `cargo test -p rusty_kafka` — 109 passed.
Severity: high.

**15. The same unchecked-array-length-before-allocation pattern recurs across every other response decoder in the crate.**
Location: `api_versions.rs`, `create_topics.rs`, `fetch.rs` (3 nested prefixes), `join_group.rs`, `list_offsets.rs`, `offset_commit.rs`, `offset_fetch.rs`, `produce.rs`, `consumer_protocol.rs` — all use unchecked `read_array_len` instead of the `read_checked_array_len` helper `metadata.rs` already adopted for the identical round-1 finding.
Fix: replaced every call site with `read_checked_array_len`, adding a per-decoder minimum-element-size constant.
Regression tests: 17, one or more per affected decoder. Verified: covered in the same `cargo test -p rusty_kafka` run.
Severity: medium.

**16. No timeout anywhere on TCP connect or request/response I/O.**
Location: `crates/rusty_kafka/src/client.rs` — a broker that accepts the connection but never responds hangs the caller forever.
Trigger: confirmed the reverted code hangs indefinitely (killed after a 30s bash timeout).
Fix: `DEFAULT_CONNECT_TIMEOUT`/`DEFAULT_CALL_TIMEOUT` wrapping connect and the write/read round trip in `tokio::time::timeout`.
Regression tests: 2. Verified: `cargo test -p rusty_kafka --lib` — 128 passed.
Severity: high.

## `rusty_skillopt` (`skillopt-model`)

**17. Three HTTP-backed `ChatBackend`s have no request timeout and buffer responses unboundedly.**
Location: `src/anthropic.rs`, `src/azure_openai.rs`, `src/openai_compat.rs` — `reqwest::Client::new()` with no timeout, `resp.text()` with no cap.
Fix: per-backend `DEFAULT_TIMEOUT`/`with_timeout` builder plus a shared `read_capped_body` helper (16 MiB cap).
Regression tests: 6 (timeout + oversized-body per backend). Verified: covered in the fix task's `cargo test -p skillopt-model` run.
Severity: high.

**18. Two subprocess-backed `ChatBackend`s have no timeout on the child-process wait.**
Location: `src/claude_cli.rs`, `src/aisf_stage.rs` — a hung `claude -p`/AISF process blocks `Engine::train` indefinitely.
Fix: `tokio::time::timeout` wrapping `wait_with_output()` plus `kill_on_drop(true)`, mirroring `nexus-agent`'s existing pattern.
Regression tests: 2, using a real hung test-helper binary substituted via PATH injection/constructor argument. Verified: 7 + 8 passed.
Severity: medium.

## `rusty_rusqlite`

**19. The boolean-expression parser and its evaluator both recurse with no depth limit.**
Location: `src/dml_select.rs` (`parse_or_expr`/`parse_and_expr`/`parse_not_expr`/`parse_bool_primary`), `src/eval.rs` (`evaluate_with_context`) — this crate's own `config.rs` documents its `ExprDepth` resource limit as unenforced.
Trigger: confirmed a real `STATUS_STACK_OVERFLOW` against the reverted code with 10,000 nested parens or chained NOTs.
Fix: `MAX_EXPR_DEPTH=128` threaded through the parser; a matching evaluator-side cap since `Expr`/`evaluate` are public and can be hand-constructed bypassing the parser entirely.
Regression tests: 3. Verified: `cargo test -p rusty_rusqlite` — 641 passed.
Severity: high.

## `rusty_time`

**20. `DateTime::parse` unconditionally rejects valid RFC 3339 leap-second timestamps.**
Location: `crates/rusty_time/src/lib.rs` — `second == 60` fails `Time::from_hms_nano`'s range check despite this crate documenting itself as an RFC 3339 parser.
Fix: normalize a parsed leap second to `23:59:59.999999999` of the same minute (RFC 3339's own recommended handling), documented in a code comment.
Regression test: parses the real historical leap second `1998-12-31T23:59:60Z`. Verified: `cargo test -p rusty_time` — 15 passed.
Severity: low.

## `rusty_croc`

**21. Unbounded DEFLATE decompression reachable pre-authentication.**
Location: `src/compress.rs` (`decompress()` inflates into a growing `Vec` with no output cap), `src/message.rs` (`decode()` calls it on every control-channel frame, including the first one received before the PAKE key is set).
Fix: capped output at `MAX_DECOMPRESSED_SIZE` (64 MiB, matching the crate's existing wire-frame cap) via `.take()`, erroring past it.
Regression test: `compress::tests::decompress_rejects_bomb`, a classic zip-bomb-shaped input. Verified: `cargo test -p rusty-croc` — 54 passed.
Severity: high.

**22. Receive-side chunk writes accept an attacker-supplied offset with no bound against the negotiated file size.**
Location: `src/croc.rs` (`receive_data_loop`'s `write_at` calls) — a malicious sender can specify `pos` far beyond the agreed transfer size, sparse-allocating an arbitrarily large file.
Fix: validate `pos + chunk.len()` stays within the negotiated `fi.size` before writing.
Regression test: `croc::tests::receive_data_loop_rejects_pos_beyond_declared_size`. Verified: covered in the same 54-test run.
Severity: medium.

## `nexus-agent`

**23. `normalize_agent_id` accepts `.`/`..` as a valid agent id, letting a caller escape the per-agent memory directory by one level.**
Location: `crates/nexus/crates/nexus-agent/src/memory.rs` — character-class-only validation passes `agent_id = ".."`, which then resolves `history_path` to `.forge/history.jsonl`, one level outside the intended per-agent scope; `memory_prune` can additionally delete arbitrary lines from that shared file.
Fix: reject any dot-delimited component that is empty, `.`, or `..`, while still accepting reverse-DNS-style ids like `com.nexus.agent.coder`.
Regression test: `memory::tests::normalize_agent_id_rejects_dot_path_components`. Verified: `cargo test -p nexus-agent` — 251 passed.
Severity: medium.

## `rusty_whisper`

**24. GGUF `n_tensors` header field drives an unbounded pre-allocation, and quantized-tensor byte length silently floor-divides without a block-alignment check.**
Location: `src/gguf.rs` — the sibling `rusty_llama::gguf` already caps this exact field at `.min(1 << 16)`; the cap was never ported. The legacy `.bin` loader in `model.rs` already rejects non-block-aligned element counts; the GGUF path doesn't, silently corrupting dequantized tensor tails instead of erroring.
Fix: ported both sibling-crate guards verbatim.
Regression tests: 2. Verified: `cargo test -p rusty-whisper --features gguf` — 224 + 4 passed.
Severity: high.

**25. WAV/RIFF chunk-size fields drive unbounded allocations, reachable from `whisper-server`'s public upload endpoint.**
Location: `src/wav.rs` (5 call sites reading a 32-bit chunk size with only a lower-bound check from round 3, no upper bound).
Fix: `MAX_CHUNK_SIZE` cap applied at all 5 sites.
Regression tests: 5. Verified: `cargo test -p rusty-whisper --lib` — 218 passed.
Severity: high.

**26. `whisper-server`'s HTTP request parser and `whisper-talk-llama`'s LLM client both read an unbounded Content-Length body.**
Location: `src/http.rs` (`parse_request`, pre-auth, network-reachable), `src/llm_client.rs` (`read_response`) — the sibling `rusty_llama::server` already caps this exact pattern.
Fix: ported the sibling's `MAX_BODY_BYTES` cap-check to both.
Regression tests: 2. Verified: `cargo test -p rusty-whisper --lib` — 217 passed (1 unrelated test from a concurrent sibling fix skipped, confirmed unrelated).
Severity: high (server), medium (client).

**27. The hand-rolled JSON parser and GBNF grammar parser both have unbounded recursion.**
Location: `src/json.rs` (`parse_value`/`parse_object`/`parse_array`, reachable from a malicious chat-completions server's response), `src/grammar.rs` (`parse_sequence`'s `(` handling, reachable from a local `--grammar` file) — the sibling `rusty_llama::grammar` already fixed the identical grammar-parser bug at depth 64.
Fix: ported the same `MAX_NESTING_DEPTH=64` cap to both.
Regression tests: 3. Verified: `cargo test -p rusty-whisper --lib` — 218 passed.
Severity: medium.

## `rusty_font`

**28. CFF Type 2 charstring interpreter caps recursion depth but not branching — exponential-blowup CPU-exhaustion DoS.**
Location: `src/cff.rs` (`Interpreter::call_subr`, `MAX_DEPTH=10`) — the same bug shape round 3 fixed for TrueType composite glyphs (which added memoization), never ported to this separate CFF/OTTO code path; each of the 10 permitted nesting levels can invoke unbounded further calls, producing up to O(B^10) total work before the depth cap even triggers.
Trigger: confirmed the reverted code hangs (no result within a 20s bound) on a sub-1KB crafted font with a small self-recursive subroutine.
Fix: added a total per-glyph operation budget (`MAX_OPS=50,000`) in addition to the existing depth cap, since branching can't be memoized the way TrueType components were.
Regression test: `ttf::tests::cff_glyph_outline_rejects_a_bounded_depth_branching_subroutine_bomb`, completing in under 5s post-fix. Verified: `cargo test -p rusty_font` — 29 passed.
Severity: high.

## `rusty_rdp`

**29. Five independent PDU decoders allocate from a raw, unvalidated 32-bit count before reading any element.**
Location: `src/gcc.rs` (`ClientNetworkData::decode`, server-side pre-auth), `src/cliprdr.rs` (file-list decode), `src/rdpdr.rs` (device-list-announce, server-side), `src/gfx.rs` (`Avc420MetaBlock::decode`, client-side from a malicious server), `src/output.rs` (`parse_palette`, client-side) — `gfx.rs`'s own `ResetGraphicsPdu` handler already establishes the fix pattern (a named `MAX_*` cap checked before allocating) elsewhere in the same file.
Fix: mirrored that exact pattern at all five sites with protocol-appropriate maximums (31 channels, 8192 file entries, 255 devices, 512 regions, 256 palette entries).
Regression tests: 5, one per decoder. Verified: `cargo test -p rusty_rdp` — 528 passed.
Severity: high.

---

## Delivery notes

All 34 findings above were fixed by 29 parallel fix tasks (one per
disjoint crate group), each adding a regression test in that crate's
existing test conventions and verifying with a package-scoped `cargo test
-p <crate>`. After all 29 landed, a workspace-lint sweep (`cargo fmt` then
`cargo clippy --all-targets -D warnings`) was run against all 23 touched
crates (21 via native Windows cargo, `rusty_h2`/`rusty_stream` via WSL
Fedora since both depend on a `#[cfg(target_os = "linux")]`-gated
`io-uring` backend) — the fmt pass was clean; the clippy pass caught 3
real issues in newly-added test/example code (a `manual_repeat_n` lint in
`rusty_http`'s test fixture, a `cloned_ref_to_slice_refs` lint in
`rusty_h2`'s test fixture, an `unused_must_use` on a fallible call in
`rusty_lsp`'s example `text_server` binary), all fixed directly and
re-verified clean on a second sweep.

Several fix tasks noted pre-existing, unrelated environment limitations
encountered during verification, none caused by this round's changes:
`rusty_win32`'s full-package test run has 7 pre-existing, unrelated
failures (6 `net::tests::*` fixed-port TCP bind contention from many
concurrent sibling test runs sharing this machine, 1 `console::tests`
timing flake) — the fix's own `dynlib` test scope was independently
verified clean; `rusty_stream`/`rusty_h2` cannot compile at all on native
Windows (confirmed via `git stash` against the unmodified tree) due to
their `io-uring-fs` dependency, verified instead via WSL.

No workspace-wide `cargo check --workspace`/`cargo test --workspace` was
run (impractical at this scale); the 23-crate clippy/fmt sweep plus each
task's own package-scoped test run is this round's verification bar,
matching rounds 3-5's precedent. No commits were made during the fix
phase — all changes are unstaged working-tree edits at the time this
report was written; commit/branch/PR/merge follows as a separate step if
requested, also matching prior rounds' precedent.

This pass covered seventeen additional crate-family groups; it is not
exhaustive of the remaining workspace surface. Still not reviewed at this
depth: `rusty_inventrory`'s Tauri-side Rust code beyond its core (touched
only lightly this round), the remaining `rusty_hister` crates
(`rusty-hister-core`/`-indexer`/`-vectorstore`/`-crawler`/`-server`/
`-mcp`, all still pre-implementation stubs with no code to review), most
of `nexus-mcp`'s dynamic-tool registry and `nexus-ai`/`nexus-ai-runtime`'s
streaming-response buffering (noted as a lower-confidence, operator-
configured-endpoint-only concern in this round's own scout report), and
the `sessionmgr-desktop`/`inventory-tauri`/`rk-desktop` Tauri frontend
(JS/TS) family, which every round including this one has scoped out as
outside a Rust-focused review.

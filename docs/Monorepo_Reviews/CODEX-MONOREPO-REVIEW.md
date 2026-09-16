# Monorepo improvement review — rusty_mill

Reviewed on 2026-09-11 against checkout HEAD `8a32cc2e925cc5072145eb966f297a0fc7f47099`.
Advisory analysis only; recommendations below are not implemented or independently
reviewed. The frozen work order's SHA-256 was verified as
`7743b06a377400e717b51a1b8a556ce972753127e42733237aaad01ce444d806`.

## Method and scope

Read `repo-inspector-report.md` first, including its 2026-09-05 dispositions.
This review does not reopen its duplication clusters, dependency replacements,
or prerequisite-dependent sovereignty work. Inspected root `Cargo.toml` (231
explicit workspace members in this checkout), the `crates/` directory structure,
root README/contributor/architecture documentation, and the active root CI
workflow, composite actions, affected-package selector, and nextest configuration.
Then read implementation and relevant test code for the findings below, covering
17 crates plus workspace CI. This is a sampled review, not an exhaustive audit of
all workspace members.

All locations are relative to this checkout; line numbers refer to the unchanged
source at the HEAD above. Evidence is from static inspection. Example inputs and
failure sequences below are proposed regression cases derived from that code,
not claims that executable reproductions were run. No Cargo build/test, network
probe, vulnerability-database scan, or performance benchmark was performed.
High severity denotes potential memory unsafety, durable data loss, credential
exposure, or hostile-input resource exhaustion; medium denotes bounded correctness,
reliability, or coverage defects; low denotes documentation or avoidable work.

## Findings

1. **Reject a lone quote instead of panicking in INI parsing.**
   **Location:** `rusty_config`, `crates/rusty_config/src/lib.rs:53`, `Config::parse` (especially lines 57–60).
   **Evidence:** Optional quote stripping tests only `starts_with` and `ends_with`. A one-byte value consisting of a single quote satisfies both checks, then attempts the invalid slice `val[1..0]`. Both `key='` and `key="` can therefore panic in a public API returning `Result`.
   **Recommendation:** Require two delimiter bytes before stripping, and return a line-specific `ParseError` for an unterminated quoted value. Add regression cases for each lone quote, valid empty quoted strings, and multibyte content.
   **Severity:** medium.

2. **Make writer patch bounds checks overflow-safe.**
   **Location:** `rusty_wire`, `crates/rusty_wire/src/lib.rs:248`, `Writer::patch_u16_be`; also `patch_u32_be` at line 260.
   **Evidence:** The guards compute `offset + 2` or `offset + 4` before comparing with buffer length. An offset such as `usize::MAX` overflows with overflow checks enabled; with wrapping arithmetic it can pass the guard and subsequently panic on the slice. This contradicts the crate's bounds-checked, error-returning interface.
   **Recommendation:** Use `checked_add` and `get_mut` for the entire destination range, returning `InvalidValue` on failure. Test maximum offsets and exact end-of-buffer writes for both widths.
   **Severity:** medium.

3. **Enforce the documented nanosecond range.**
   **Location:** `rusty_time`, `crates/rusty_time/src/lib.rs:100`, `Time::from_hms_nano`; `nanosecond` documentation at line 132.
   **Evidence:** Construction rejects invalid hours, minutes, and seconds but stores any `u32` nanosecond value. `from_hms_nano(0, 0, 0, 1_000_000_000)` succeeds even though the accessor documents a maximum of `999_999_999`.
   **Recommendation:** Reject `nano >= 1_000_000_000` through the existing error path. Add boundary cases for the maximum valid value, one billion, and `u32::MAX`.
   **Severity:** medium.

4. **Preserve the instant and fractional precision when formatting datetimes.**
   **Location:** `rusty_time`, `crates/rusty_time/src/lib.rs:181`, `DateTime::to_iso8601`; `timestamp` at line 172 and parse tests at lines 345–355.
   **Evidence:** Formatting always appends `Z` to the stored local date/time and never reads `offset_secs` or nanoseconds. Parsing `2026-08-12T01:18:55.5+02:00` and formatting it produces `2026-08-12T01:18:55Z`, changing the represented instant by two hours and losing the fraction. The formatting round-trip test uses only a whole-second UTC value.
   **Recommendation:** Emit the stored offset and fractional seconds, or convert to UTC before emitting `Z` while retaining precision. Test parse/format preservation of timestamp, fraction, negative offsets, and date-boundary crossings.
   **Severity:** medium.

5. **Validate timezone components before converting them to seconds.**
   **Location:** `rusty_time`, `crates/rusty_time/src/lib.rs:249`, offset branch in `DateTime::parse`.
   **Evidence:** The parser reads two-digit offset hours and minutes and immediately computes `offset_hour * 3600 + offset_minute * 60`. Unlike the time-of-day constructor, it checks neither component's range; strings ending in `+00:99` or `+99:00` are accepted by an API documented as parsing RFC 3339.
   **Recommendation:** Validate offset hours and minutes before conversion and reject out-of-range components. Add explicit valid-boundary and invalid-component tests, separately from time-of-day validation.
   **Severity:** medium.

6. **Reset transaction state before returning a connection to the pool.**
   **Location:** `rusty_sqlite`, `crates/rusty_sqlite/src/pool.rs:102`, `PooledConnection::drop`; raw connection access at lines 111–121.
   **Evidence:** A lease exposes `rusqlite::Connection`, allowing `execute_batch("BEGIN; ...")`. Dropping the lease only pushes that connection into `idle`; it does not roll back an open SQL transaction. The next borrower can inherit uncommitted changes and locks, or accidentally commit the previous borrower's work.
   **Recommendation:** Check transaction/autocommit state on return and roll back unfinished transactions. Discard a connection and release its pool slot if cleanup fails. Add a one-connection pool regression where a lease is dropped after `BEGIN` and a write, then reacquired.
   **Severity:** high.

7. **Reject a zero-capacity connection pool at construction.**
   **Location:** `rusty_sqlite`, `crates/rusty_sqlite/src/pool.rs:142`, `build_pool_with_timeout`; acquisition loop at lines 57–98.
   **Evidence:** The fallible builder accepts `max_size = 0`. Such a pool has no idle connections and can never satisfy `state.total < max_size`, so every acquisition waits until its timeout even though success is structurally impossible.
   **Recommendation:** Return a configuration error for zero capacity, or use a nonzero capacity type. Add a constructor-level test so invalid configuration fails immediately without waiting for the acquisition timeout.
   **Severity:** medium.

8. **Validate migration versions individually, not just pairwise ordering.**
   **Location:** `rusty_sqlite`, `crates/rusty_sqlite/src/migration.rs:7`, version contract; `Migrations::validate_order` at line 50 and `run` at line 79.
   **Evidence:** Documentation requires positive versions, but validation only visits `steps.windows(2)`. A migration at version zero followed by version one passes validation; on a fresh database, the version-zero SQL is silently skipped by `m.version > current`. A single zero-version migration is likewise accepted and skipped.
   **Recommendation:** Validate every version as positive before checking ordering, returning a dedicated error for invalid versions. Test a single zero, a negative first version, and a zero followed by a positive version.
   **Severity:** medium.

9. **Reject conflicting repeated Content-Length headers.**
   **Location:** `rusty_http`, `crates/rusty_http/src/body.rs:50`, `request_framing`, and line 64, `response_framing`; `crates/rusty_http/src/header.rs:75`, `HeaderMap::get`; `crates/rusty_http/src/head.rs:199`, `parse_header_lines`.
   **Evidence:** Head parsing appends repeated header names. Both framing functions then use `get("content-length")`, which returns only the first value. Headers containing lengths `1` and `100` are accepted as length one without inspecting the conflict. Differing interpretations by adjacent HTTP components are a message-boundary risk.
   **Recommendation:** Validate all Content-Length occurrences before selecting framing, rejecting conflicting or malformed values and defining handling of identical repetitions. Add raw-head-to-framing tests with reversed duplicate order and conflicting lengths.
   **Severity:** high.

10. **Apply chunk framing limits to complete lines as well as incomplete ones.**
    **Location:** `rusty_http`, `crates/rusty_http/src/body.rs:208`, `ChunkedDecoder::advance`, especially lines 211–218 and 251–259; limit helper at line 267.
    **Evidence:** `max_line_len` is checked only when `next_line` returns no line. A complete over-limit chunk extension or trailer line supplied in one buffer is accepted, while the same bytes split before the newline can fail. The limit is therefore dependent on input fragmentation.
    **Recommendation:** Check the consumed line length on every successful line parse as well as on incomplete buffers. Add paired tests supplying an oversized line whole and split, requiring the same rejection in both cases.
    **Severity:** medium.

11. **Avoid truncating valid large records during segment recovery.**
    **Location:** `rusty_stream`, `crates/rusty_stream/src/segment.rs:104`, `Segment::open_on`; unrestricted append entry point at line 181.
    **Evidence:** Each recovery iteration reads at most 64 KiB and decodes that single buffer. `PayloadTruncated` immediately causes `file.set_len(pos)`. `append` does not impose that record-size ceiling, so a complete, successfully written record whose encoding exceeds 64 KiB is classified as torn on reopen and removed together with following records. A short read can create the same problem for smaller records.
    **Recommendation:** Read a complete record header, validate its declared size against an explicit policy, then fill the required payload before deciding whether EOF means a torn record. Add recovery regressions around 64 KiB and with a driver that returns short reads.
    **Severity:** high.

12. **Finish segment writes before acknowledging and indexing them.**
    **Location:** `rusty_stream`, `crates/rusty_stream/src/segment.rs:63`, `Segment::create_on`, and line 184, `Segment::append`; supporting implementation in `crates/rusty_tokio/src/io/uring_fs.rs:1380`, `UringFile::write_at`.
    **Evidence:** Header creation ignores the returned byte count. Append treats any successful count as a complete record, advances `write_pos` by that count, and adds an index entry. `UringFile::write_at` submits once and returns the driver's count without a write-all loop. A successful short write can thus be acknowledged as a valid record and later discarded during recovery.
    **Recommendation:** Loop until the full header/record has been written, handling zero progress as an error, and publish the index entry only after completion. Define recovery after a partially completed error and add short-write/zero-write driver tests.
    **Severity:** high.

13. **Validate consumer IDs before encoding their length.**
    **Location:** `rusty_stream`, `crates/rusty_stream/src/consumer.rs:32`, `encode_commit`; public `ConsumerOffsets::commit` at line 97 and decoder length check at line 52.
    **Evidence:** Encoding casts the ID's byte length to `u16` but writes the entire ID. `commit` applies no length validation before appending. An ID of 65,536 ASCII bytes gets a zero length prefix and a record that `decode_commit` rejects on reopening, making the offset store unrecoverable through its normal open path.
    **Recommendation:** Check the UTF-8 byte length with `u16::try_from` before any append or in-memory mutation. Test the largest valid ID and one byte beyond it, including a multibyte ID whose character count hides an excessive byte count.
    **Severity:** high.

14. **Enforce owner-only permissions when overwriting an existing Unix secret file.**
    **Location:** `rusty_crypto_key`, `crates/rusty_crypto_key/src/lib.rs:83`, `SecretBytes::save_to_file`, especially lines 97–104.
    **Evidence:** The method promises restricted permissions when creating or truncating a file. It supplies creation mode `0600`, but opens existing files with `create(true).truncate(true)` and does not inspect or change their permissions. Creation mode does not tighten an existing file, so overwriting a readable existing file can expose the secret despite the stated contract. The separately documented Windows limitation is not this finding.
    **Recommendation:** Adopt a deliberate overwrite policy that guarantees restrictive permissions before secret bytes are written, such as a securely created replacement file and atomic replacement. Account for existing symlinks explicitly. Add a Unix regression starting with an existing `0644` file.
    **Severity:** high.

15. **Validate public audio conversion parameters.**
    **Location:** `rusty_audio`, `crates/rusty_audio/src/lib.rs:64`, `resample_linear`, and line 88, public `resample_to_mono`.
    **Evidence:** The public function accepts arbitrary source rate, target rate, and channel count. With nonempty samples, `from_rate = 0` and positive `to_rate` produce a zero ratio, infinite calculated output length, and a `Vec::with_capacity` request at the saturated integer maximum. Zero channels also pass through as if the input were mono.
    **Recommendation:** Provide a checked conversion API that rejects zero rates/channels before arithmetic or allocation and checks the computed output size. Add zero-parameter cases and a sensible output-size boundary test.
    **Severity:** medium.

16. **Preserve permanent WASAPI errors at the capture API boundary.**
    **Location:** `rusty_audio`, `crates/rusty_audio/src/lib.rs:123`, `AudioCapture::read_samples`; `crates/rusty_audio/src/wasapi.rs:512`, unsupported-format branch.
    **Evidence:** The wrapper calls `self.inner.read_samples().unwrap_or_default()`, converting every error into an empty sample vector. The backend explicitly returns an error for unsupported formats such as 24-bit PCM, so a permanent failure is indistinguishable from no available audio. This also contradicts the crate-level statement at lines 15–17 that unsupported formats fail loudly.
    **Recommendation:** Return a typed `Result` from the wrapper, or add a checked method and clearly document any lossy convenience method. Preserve permanent format errors; test their propagation separately from an empty successful read.
    **Severity:** medium.

17. **Reject strings that exceed Kafka's signed length field.**
    **Location:** `rusty_kafka`, `crates/rusty_kafka/src/wire.rs:73`, `write_nullable_string`; public caller `crates/rusty_kafka/src/protocol/metadata.rs:21`, `MetadataRequest::encode`.
    **Evidence:** The encoder casts `text.len()` to `i16` and then appends every byte. A 32,768-byte topic string becomes a negative length followed by a full payload; the decoder in the same file rejects lengths below `-1`. Public request fields and encoding do not prevent this malformed output.
    **Recommendation:** Make string encoding fallible and propagate checked length conversion through request encoders, validating before partially writing a request. Test 32,767 and 32,768 UTF-8 bytes. Audit the analogous byte/array length casts as part of that fix.
    **Severity:** medium.

18. **Bound decoded Kafka collection counts before allocating.**
    **Location:** `rusty_kafka`, `crates/rusty_kafka/src/protocol/metadata.rs:84`, `MetadataResponse::decode`, especially lines 85–86, 95–102, and 108–115.
    **Evidence:** Broker, topic, partition, replica, and ISR counts are fed directly into `Vec::with_capacity` before validating whether the reader contains enough bytes for those elements. A four-byte count of `i32::MAX` can request a huge allocation before the first element read fails. The outer frame-size cap in `crates/rusty_kafka/src/frame.rs:36` does not bound this decoded allocation amplification.
    **Recommendation:** Bound each count by remaining encoded bytes and a configured collection limit before reserving; use fallible allocation where appropriate. Test huge counts in tiny payloads at each nesting level and require a codec error rather than allocation failure.
    **Severity:** high.

19. **Truncate fetched text on a UTF-8 character boundary.**
    **Location:** `rk-feed`, `crates/rusty_key/crates/feed/src/web.rs:169`, `web_fetch_impl`; `FETCH_CAP` at line 14.
    **Evidence:** After UTF-8-safe HTML stripping, the caller executes `text.truncate(50_000)` whenever the byte length exceeds the cap. `String::truncate` requires a character boundary. Text consisting of 49,999 ASCII bytes followed by a multibyte character crosses that boundary and can panic the fetch tool.
    **Recommendation:** Find a character boundary at or below the byte cap before truncation. Add fetched-content cases with two-, three-, and four-byte characters crossing the cap, and retain the truncation marker.
    **Severity:** medium.

20. **Enforce the web-fetch size cap while receiving the body.**
    **Location:** `rk-feed`, `crates/rusty_key/crates/feed/src/web.rs:161`, `web_fetch_impl`; `strip_html` at lines 106–110 and 145–148.
    **Evidence:** Fetching calls `.text().await` to collect the complete response before applying `FETCH_CAP`. HTML stripping then makes additional input-sized allocations. A large response therefore consumes memory proportional to its full size even though the tool returns only about 50 KB; the 30-second timeout is not a byte budget.
    **Recommendation:** Read the response incrementally with a hard received/decoded body budget before constructing the full string or invoking HTML processing. Reject or explicitly mark oversized responses, including chunked responses without Content-Length. Add a bounded local-server regression.
    **Severity:** high.

21. **Remove expired cache entries from the eviction queue as well as the map.**
    **Location:** `rp-router`, `crates/rusty_provider/crates/router/src/cache.rs:63`, `ResponseCache::get`, and line 72, `insert`.
    **Evidence:** Expiry removes a key from `entries` but leaves it in `order`. With capacity two: insert A and B, expire/get B, then reinsert B. The queue becomes `[A, B, B]`; insertion pops A, leaving two references to B. Inserting C then pops the older B reference and removes the newly refreshed B from the map, leaving only C even though capacity permits both B and C.
    **Recommendation:** Keep queue membership synchronized on expiry/reinsertion, or attach generation tokens so stale queue entries cannot evict newer values. Add a deterministic expiry/reinsert/evict regression using controlled timestamps.
    **Severity:** medium.

22. **Build session summaries without cloning their discarded histories.**
    **Location:** `adk-sessions`, `crates/rusty_adk/crates/adk-sessions/src/in_memory.rs:132`, `InMemorySessionService::list_sessions`.
    **Evidence:** The comment says listings omit history to avoid materializing every event, but lines 141–142 first clone the entire `Session` and then clear `summary.events`. Every event payload is cloned and immediately dropped while the store's global mutex remains held.
    **Recommendation:** Construct a summary from metadata/state fields with an empty event vector, or add a dedicated summary type/helper. Verify that listing preserves metadata and hydrated state; use a long-history allocation benchmark to measure the removed copying.
    **Severity:** low.

23. **Handle two empty inputs in the Myers diff implementation.**
    **Location:** `rusty_diff`, `crates/rusty_diff/src/lib.rs:11`, `diff_myers`, especially lines 16 and 21–30; tests at line 188.
    **Evidence:** Empty inputs give `max_d = 0` and a one-element `v`. The first `d = 0, k = 0` iteration takes the branch reading `v[k + 1 + offset]`, i.e. `v[1]`, and panics. Formatting a diff between two empty strings reaches the same path. Existing tests use nonempty inputs.
    **Recommendation:** Return an empty operation list for two empty inputs and ensure the frontier has the sentinel space required for other boundary cases. Add empty/empty, insertion-only, and deletion-only regressions.
    **Severity:** medium.

24. **Honor hunk coordinates when applying unified patches.**
    **Location:** `rusty_diff`, `crates/rusty_diff/src/lib.rs:136`, `apply_patch`, especially lines 139–145 and 150–163.
    **Evidence:** Every `@@` header is skipped, while `orig_idx` always begins at zero. A valid hunk beginning at original line three is compared with line one instead of first copying the untouched prefix. A later repeated line can also be changed at the wrong occurrence because the declared location is ignored. The round-trip test only exercises this crate's full-file hunk formatter.
    **Recommendation:** Parse and validate hunk start/count fields, copy untouched spans, and apply each hunk at its declared source position. Add externally shaped partial-context and multiple-hunk fixtures, plus inconsistent-count rejection cases.
    **Severity:** medium.

25. **Distinguish patch file headers from hunk lines beginning with repeated signs.**
    **Location:** `rusty_diff`, `crates/rusty_diff/src/lib.rs:143`, header-skipping branch in `apply_patch`.
    **Evidence:** Any line starting with `---` or `+++` is skipped anywhere in the patch. Within a hunk, adding text `++value` produces the patch line `+++value`, which this function discards as a header; deleting text `--value` is similarly ignored. This can break patches emitted by `format_unified_diff` itself, independently of hunk-position support.
    **Recommendation:** Track whether parsing is in the file-header or hunk-body state; interpret the first byte of every hunk-body line as its operation. Add round-trip cases for content beginning with two or more plus/minus signs.
    **Severity:** medium.

26. **Reject a DEFLATE stream that ends before a final block.**
    **Location:** `rusty_compress`, `crates/rusty_compress/src/lib.rs:74`, `decompress_deflate`, especially lines 78, 112–117; test at line 125.
    **Evidence:** Leaving the loop because the input buffer is exhausted returns `Ok(out)` even when no block had its final bit set. Both an empty byte slice and `[0x00, 0x00, 0x00, 0xff, 0xff]` (an empty non-final stored block with no successor) are accepted. The one round-trip test always uses the encoder's correctly finalized output.
    **Recommendation:** Require encountering a final block before reporting success; otherwise return `TruncatedInput`. Add truncated-at-block-boundary, empty-input, and valid-empty-final-block tests.
    **Severity:** medium.

27. **Document the actual stored-block-only compression surface.**
    **Location:** `rusty_compress`, `crates/rusty_compress/src/lib.rs:1`, crate documentation; `CompressionLevel` at line 9, `compress_deflate` at line 40, and `decompress_deflate` at lines 89–109; `crates/rusty_compress/README.md:5`.
    **Evidence:** The crate and README advertise DEFLATE, Gzip, and Zlib helpers, but the entire implementation exposes only raw DEFLATE functions. The encoder ignores `_level` and emits stored blocks; the decoder returns `CorruptData` for compressed block types. Callers can mistake unsupported valid compressed data for corruption and assume the named levels affect compression.
    **Recommendation:** Narrow public documentation to raw stored-block DEFLATE, state explicitly that levels currently have no effect, and distinguish unsupported encoding from corrupt input. Describe wrapper formats as future scope rather than shipped functionality. Add a fixture demonstrating the unsupported-format error contract.
    **Severity:** low.

28. **Avoid overwriting explicit document IDs during automatic ID assignment.**
    **Location:** `rusty-search-memory`, `crates/rusty_search/crates/rusty-search-memory/src/lib.rs:76`, `MemoryBackend::index_batch`.
    **Evidence:** ID-less documents receive `_auto_1`, `_auto_2`, etc., without checking existing keys; explicit IDs do not advance the counter. Inserting an explicit `_auto_1` and then an ID-less document causes `HashMap::insert` to replace the explicit document silently. The public API does not reserve this namespace here.
    **Recommendation:** Choose a generated ID that is absent from the index, checking within the same write lock, or enforce a documented reserved namespace for explicit IDs. Add mixed explicit/generated-ID tests both within one batch and across batches.
    **Severity:** medium.

29. **Constrain the spinlock guard's automatically derived Sync behavior.**
    **Location:** `rusty_sync`, `crates/rusty_sync/src/lib.rs:31`, unsafe `Send`/`Sync` implementations for `SpinLock<T>`; `SpinLockGuard` at line 53 and `Deref` at line 57.
    **Evidence:** `SpinLock<T>` is `Sync` when `T: Send`, which is appropriate for a mutex. However, its guard contains only `&SpinLock<T>`, so the guard also automatically becomes `Sync` for `T: Send` without requiring `T: Sync`. Sharing a guard for `Cell<u32>` between scoped threads exposes shared `&Cell<u32>` through `Deref`, allowing unsynchronized interior mutation in safe client code.
    **Recommendation:** Control guard auto traits explicitly, using an appropriate marker and bounds so shared guard access requires `T: Sync` while any supported guard movement has separately justified bounds. Document the unsafe invariants. Add compile-fail trait-bound tests for `SpinLockGuard<Cell<_>>` and positive tests for safe element types; use concurrency tooling for runtime paths.
    **Severity:** high.

30. **Rank borrowed RAG documents before cloning the requested hits.**
    **Location:** `rusty_rag`, `crates/rusty_rag/src/lib.rs:58`, `SearchIndex::search_vector`, especially lines 68–79; analogous keyword search at lines 105–119.
    **Evidence:** Vector search clones every document, including full content and embedding buffers, sorts all clones, then truncates to `limit`. Even `limit = 0` clones the entire index. Keyword search does the same for all matching documents before discarding excess hits. The allocation/copy cost is visible directly in the implementation, without assuming SIMD performance characteristics.
    **Recommendation:** Rank references or indices, return early for zero limit, and clone only selected results. Consider bounded top-k selection if measured index sizes justify it. Compare allocations for a large-content index with limits zero, one, and many while checking result equivalence.
    **Severity:** low.

31. **Exclude incompatible embeddings from vector-search results.**
    **Location:** `rusty_rag`, `crates/rusty_rag/src/lib.rs:61`, `SearchIndex::search_vector`.
    **Evidence:** A document with an empty embedding or a different dimension gets score zero and is still inserted into the results. A valid dimension-matched document with a negative dot product ranks below these unscored documents. For query `[1.0]`, an incompatible embedding can outrank the valid embedding `[-1.0]` solely because the fallback score is zero.
    **Recommendation:** Skip incompatible embeddings or reject inconsistent dimensions through an explicit index/query validation contract. Add tests mixing missing, mismatched, negative-scoring, and valid embeddings so invalid candidates cannot displace actual matches.
    **Severity:** medium.

32. **Treat nextest configuration changes as affecting test execution.**
    **Location:** workspace CI, `.github/workflows/ci.yml:113`, `plan` job; `.github/scripts/affected_crates.py:27`, `owning_crate`, and line 89, empty-selection handling; `.config/nextest.toml:27`.
    **Evidence:** The workflow's full-sweep pattern includes root Cargo files and `.github` workflow/action/script paths, but not `.config/nextest.toml`. That file is outside all crate directories, so a PR changing only its retry/timeout configuration produces no affected packages and skips build/test/clippy. The configuration governing all crate tests is therefore not exercised by that PR's test job.
    **Recommendation:** Include nextest configuration in workspace-wide test triggers, and add a planner/workflow regression for a nextest-only change. Keep documentation-only skip behavior separate from executable test configuration.
    **Severity:** medium.

33. **Exercise supported feature-off builds in root CI.**
    **Location:** workspace CI, `.github/workflows/ci.yml:199`, Clippy; build/test commands at lines 233 and 270. Concrete affected surface: `rusty_config`, `crates/rusty_config/Cargo.toml:14`, and `crates/rusty_config/src/lib.rs:6`.
    **Evidence:** The main Rust gates uniformly use `--all-features`. `rusty_config` defaults to `std`, and its `no_std` attribute plus `alloc` imports are enabled only when `std` is absent. No root workflow command exercises that advertised alternative branch; enabling all features cannot validate it. This is a feature-coverage gap even if the current branch happens to compile.
    **Recommendation:** Add an affected-package-aware matrix for specifically supported feature combinations, starting with a targeted `cargo check -p rusty_config --no-default-features`. Extend it to other documented `no_std` packages after enumerating their valid combinations, rather than applying one unsupported flag set to the entire workspace.
    **Severity:** medium.

## Delivery notes

Only this report is added. Existing source, the prior review, and repository
configuration are unchanged. No commits, pushes, publication, issues, or PRs were
performed. No work-order requirement required redesign; the 231-member inventory
is an observed checkout update within the specified 183+ scope.

Initial PowerShell profile startup attempted writes to Terminal-Icons preferences
and the user PATH registry, which the sandbox denied. Subsequent commands disabled
profile loading. Those incidental startup failures did not block the report or
require changes outside this checkout. A missing root `rust-toolchain.toml` and an
initially guessed source filename were resolved by inspecting the actual files;
neither is used as evidence of a defect on its own.

The agreed numbered-item proof is run after writing this file. Its count validates
the deliverable's size, not the correctness of individual findings; independent
review of the evidence and proposed changes remains required.

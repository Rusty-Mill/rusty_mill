# Rust adherence audit: `rusty_multimodal_db`

| | |
|---|---|
| Crate | `rusty_multimodal_db` (one package: lib, 9 bins, 5 examples, 2 benches, 44 integration test files) |
| Path | `crates/apps/rusty_multimodal_db` |
| Commit audited | `f80ee7fc5572de65928be3f7df019ae1f056c7e3` |
| Size | 111 `.rs` files, 80,926 lines. `src/` is 54,198 lines, of which 32,075 non-blank lines are outside `#[cfg(test)]`. `tests/`, `benches/`, `examples/` are 26,728 lines. |
| Tests | 506 `#[test]` in `src/`, 346 in `tests/`. No `#[ignore]`, no `#[should_panic]`, no property or fuzz tests. |
| Async | None. Zero `async` in the crate. |

All paths below are relative to `crates/apps/rusty_multimodal_db/` unless they start with `crates/`.

## How counts were produced

Lint counts come from clippy on the pinned toolchain, lib and bins only, so `#[cfg(test)]` code, `tests/`, `examples/` and `benches/` are **excluded** from every "library code" number:

```
cargo clippy -p rusty_multimodal_db --all-features --lib --bins --message-format=json -- \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -W clippy::indexing_slicing \
  -W clippy::too_many_lines -W clippy::cognitive_complexity -W clippy::too_many_arguments \
  -W clippy::missing_errors_doc -W clippy::missing_panics_doc -W missing_docs \
  -W clippy::cast_possible_truncation -W clippy::unreachable -W clippy::todo -W clippy::unimplemented \
  -W clippy::print_stdout -W clippy::print_stderr -W clippy::let_underscore_must_use \
  -W clippy::fn_params_excessive_bools -W clippy::struct_excessive_bools -W clippy::excessive_nesting \
  -W clippy::undocumented_unsafe_blocks -W unsafe_code -W clippy::map_err_ignore \
  -W clippy::wildcard_enum_match_arm -W clippy::exit -W clippy::missing_const_for_fn \
  -W clippy::type_complexity -W clippy::arithmetic_side_effects -W clippy::cast_sign_loss \
  -W clippy::cast_possible_wrap -W clippy::string_slice -W clippy::match_same_arms \
  -W clippy::must_use_candidate -W clippy::unwrap_in_result -W clippy::panic_in_result_fn \
  -W clippy::allow_attributes_without_reason -W clippy::mem_forget -W clippy::large_enum_variant \
  -W clippy::needless_pass_by_value
```

- Messages were de-duplicated on (lint, file, line, column) and filtered to files under this crate's path. The run also printed hits in path dependencies; those are not counted.
- `clippy::excessive_nesting` and the 60-line `too_many_lines` figure needed a threshold, so a second run used `CLIPPY_CONF_DIR` pointing at a scratch `clippy.toml` (`excessive-nesting-threshold = 4`, `too-many-lines-threshold = 60`). No file in the repository was changed. `--fix` was never passed.
- Non-test line counts and the adapter-similarity figures come from a scratch script that blanks every `#[cfg(test)]` item and compares normalised code lines with `difflib`. Marked "~" where approximate.
- "bins" splits into five server binaries (`dog_server`, `reminder_server`, `entity_server`, `memory_server`, `fair_play_server`) and four `research`-gated harness binaries (`crash_writer`, `crash_safety_harness`, `multiprocess_writer`, `multiprocess_harness`). The harnesses are prototypes; their `expect`/`println!` are counted but not held against preference 1.

## What was read

Read in full (the non-test part): `src/lib.rs`, `src/server/mod.rs`, `src/server/framing.rs`, `src/server/data_lock.rs`, `src/server/pem.rs`, `src/server/exposure.rs`, `src/server/journal.rs`, `src/server/changelog.rs`, `src/server/snapshot.rs`, `src/durability/mod.rs`, `src/durability/mmap_store.rs` (code, lines 147-748), `Cargo.toml`.

Read in large part: `src/server/serve.rs` (about a quarter of its 6,032 non-test lines: the `ConnectionStore` trait outline, table-file copy and backup, `ServeOptions`, authentication, rate limit, `TlsConfig`, the start and the session arms of `handle_connection`, `serve_tables`, `refuse_busy`, the aggregate reducer), `src/production.rs` (lines 180-458), `src/bin/memory_server.rs` (env parsing and most of `main`), `src/server/metrics_http.rs`, `src/server/audit.rs` (sinks).

Sampled by grep and lint hit: `src/server/memory.rs`, `entity.rs`, `relation.rs`, `fair_play.rs`, `client.rs`, `protocol.rs`, `mvcc.rs`, `sql.rs`, `dog.rs`, `order.rs`, `employee.rs`, `reminder.rs`, `access.rs`, the other four server bins, `src/test_support.rs`. `tests/` was inventoried and three helper blocks opened; test bodies were not read.

Not read: `src/server/strict.rs`, `nullable.rs`, `changelogged.rs` (beyond 20 lines), `src/durability/record_blob.rs`, `src/generic/**`, `src/generic_spike/**`, `src/store/**`, `src/concurrency/**`, `src/generator.rs`, `src/bench_support.rs`, `benches/`, `examples/` (except one function), `clients/python/`, `scripts/`. Findings about those files come from lint output only.

History: all 148 commit subjects, and the full bodies of 30 commits (the fix, review-fix and CI-fix commits). Two diffs were read (`683a9abc0`, `9279242df`). No review report, ADR body or design document was used as a source of findings.

Three findings below are marked "by reading": they follow from the code but were not reproduced by running it.

---

## A. Adherence and drift per preference

### 1. `Result` and `?` over panics; no `unwrap`/`expect` outside tests and prototypes

Adheres:
- The server library has **no `panic!`** (0 hits in lib) and 4 `.unwrap()`. Storage and wire code returns typed errors: `JournalError` (`src/server/journal.rs:152`), `ChangeLogError` (`src/server/changelog.rs:65`), `DataDirLockError` (`src/server/data_lock.rs:43`), `FrameError` (`src/server/framing.rs:22`).
- Oversized frames are refused before allocation (`src/server/framing.rs:86-89`).
- The accept loop uses `thread::Builder::spawn` and handles the error (`src/server/serve.rs:5747`, fixed by `48d48cb6a`).

Drifts:
- **46 `expect` in library code.** 16 are `expect(LOCK_POISONED)` in `src/production.rs` (for example `:295`, `:343`, `:379`), so one panic while the write lock is held turns every later call into a panic. 4 `.lock().unwrap()` in `src/server/mvcc.rs:328,336,347,494`.
- 12 `expect("just inserted")`/`expect("just replaced")` in the request path: `src/server/memory.rs:1629,1661,1696`, `src/server/entity.rs:1204,1235,1270`, `src/server/relation.rs:1358,1390,1427`, plus `results_cell ... .expect(...)` at `memory.rs:727`, `entity.rs:595`, `relation.rs:562`.
- 5 `expect` and 1 `unreachable!` in the aggregate reducer that a type could remove: `src/server/serve.rs:2510,2514,2523,2524,2541` (see preference 2).
- 3 `unreachable!` in the client's SQL path: `src/server/client.rs:1544,1667,1939`.
- Public API panics: `serve_tables` asserts `primary < tables.len()` (`src/server/serve.rs:5672`, documented under `# Panics`) and checks unique table names only with `debug_assert!` (`:5679`).
- `From<Vec<DogRecord>> for ProductionStore` creates a temp directory and an mmap file and panics if either fails (`src/production.rs:432-445`, `:447-455`, `ConcurrentStore::new` at `:406-413`).
- **Server binaries report every startup error by panicking**: 58 `panic!` across the five server `main`s (25 in `src/bin/memory_server.rs`, for example `:439`, `:450`, `:786`), plus `assert!` at `:842`. `open_stores` returns `Result<_, String>` (`:978`) and its caller does `unwrap_or_else(|e| panic!("{e}"))` (`:496`).
- The harness binaries (prototypes) hold 100 of the 110 bin `expect`s.

### 2. Ownership and types; illegal states unrepresentable; immutability

Adheres:
- Outcomes are enums, not booleans or sentinels: `InsertOutcome`, `LinkOutcome`, `ReplaceOutcome`, `DeleteOutcome`, `ReplaceIfOutcome` (`src/server/serve.rs:53-99`), `DrainOutcome` (`:5543`), `Exposure` (`src/server/exposure.rs:39`).
- RAII guards release slots, turns and snapshots on every exit: `TurnGuard` (`src/server/journal.rs:603-617`), `InFlightGuardOwned` (`src/server/serve.rs:5778`), `SnapshotGuard` (`:4581`), `MvccReleaseGuard` (`:4594`), `DataDirLock` (`src/server/data_lock.rs:60`).
- `JournalEntry<'a>` borrows batches instead of cloning (`src/server/journal.rs:93-97`).

Drifts:
- **A field that is required for four of five variants is an `Option`.** `AggregateSpec { func, field: Option<FieldRef> }` (`src/server/protocol.rs:544-547`); the reducer `expect`s it four times (`src/server/serve.rs:2510-2524`) and `numeric_i64` ends in `unreachable!` (`:2541`).
- **Session state is ten parallel locals** in `handle_connection`: `session`, `session_writes`, `staged_bytes`, `strict_commit`, `read_your_writes`, `validate_on_stage`, `snapshot_reads`, `mvcc_snapshot`, `staged_snapshot`, `failures` (`src/server/serve.rs:4725-4769`). They are reset by hand at five sites, each with a different subset (`:4996-5003`, `:5055-5072`, `:5077-5084`, `:5156-5159`, `:5176-5177`). Combinations that must not occur (a staged record write inside an MVCC session) are excluded by a match guard at `:5261-5263`, not by the type.
- `SessionOptions` is five independent `bool`s (`src/server/client.rs:527-533`), but strict commit must stand alone: the server refuses any other bit with it (`src/server/serve.rs:5039-5045`).
- `#[derive(Default)]` on `ServeOptions` (`src/server/serve.rs:3011`) gives `drain_timeout = 0`, while `new()` and `from_env()` set 30 s (`:3193`, `:3377`). `ServeOptions::default().with_shutdown(..)` therefore drains with a zero deadline (`:5763`). `ServeOptions::default()` is used 123 times across `src/` and `tests/`.
- Primitive aliases and bare tuples carry meaning: `pub type RecordId = Uuid` and `pub type FieldRef = u16` (`src/server/protocol.rs:270,277`); `ServedTables = Vec<(String, Arc<dyn ConnectionStore>)>` plus a separate `primary: usize` (`src/server/serve.rs:119`); a change-log position travels as `Option<(u64, u64)>` (`src/server/snapshot.rs:25,31,36`) although `LogPosition` exists (`src/server/changelog.rs:92`); `ManifestFile = (String, u64, [u8; 32])` (`src/server/snapshot.rs:28`).
- Boolean mode parameters: `write_batch(&ops, true)` (`src/server/serve.rs:5164`, declared `:569`), `release: bool` (`:2914`), `reduce_extreme(.., false, ..)`/`(.., true, ..)` (`:2523-2524`), `append(&ops, true)` (`src/server/changelog.rs:304,331`).
- Tokens are plain `String`s (see preference 8).
- Unchecked arithmetic on wire integers: `after + 1` on a client `u64` (`src/server/changelog.rs:414`, reached from `Request::FetchSince` at `src/server/serve.rs:4367-4372` with no prior bound; again at `:4375`). `u64::MAX` panics the connection thread in a debug build and wraps in release. `Sum`/`Avg` add stored `i64`s with `.sum()` (`src/server/serve.rs:2511,2519`), which wraps silently in release. Neither build profile sets `overflow-checks`.

### 3. Async only where there is real I/O concurrency

Adheres fully. There is no `async` and no runtime dependency. The server is one OS thread per connection over blocking `Read`/`Write` (`src/server/serve.rs:5747`). The costs of that choice are in the open: a 20 ms accept poll to see a shutdown flag (`:5563`, `:5704-5716`), a 50 ms poll thread for signals (`src/bin/memory_server.rs:406-411`), and `thread::sleep` in the drain wait (`src/server/serve.rs:5764-5766`).

### 4. Explicit, flat control flow, short functions

Adheres: small modules are flat and guard-first (`src/server/exposure.rs:87-98`, `src/server/data_lock.rs:75-94`, `src/server/pem.rs:50-83`).

Drifts:
- `handle_connection` is **647 code lines** (921 with comments, `src/server/serve.rs:4607-5528`), cognitive complexity 109 against clippy's 25. Arm order in its one `match` is load-bearing: 32 `if negotiated < N` guards and several `if session.is_some()` guards decide which arm a request reaches (`:5248-5300`).
- `main` in `memory_server` is 339 lines, complexity 49 (`src/bin/memory_server.rs:417`). `dispatch` is 258 lines (`src/server/serve.rs:4080`).
- 6 functions exceed 100 lines, 29 exceed 60.
- 109 blocks nest deeper than four levels: `memory.rs` 18, `entity.rs` 16, `relation.rs` 15, `client.rs` 13, `serve.rs` 10.
- `src/server/serve.rs` is one 11,359-line file (6,032 non-test). The `ConnectionStore` trait alone runs `:121-858`.

### 5. Composition; no abstraction before two call sites

Adheres in one direction: the crate does not invent abstractions early. `TransactionalStore` and `AllIds` each state why they are separate from `DogStore` (`src/production.rs:313-365`).

Drifts in the other direction, abstraction withheld long after the second call site:
- The three front-door adapters are near copies. After normalising domain names, `memory.rs` and `relation.rs` share ~88% of the smaller file's code lines, `memory.rs`/`entity.rs` ~86%, `entity.rs`/`relation.rs` ~80% (~1,100 lines each). `dog`/`order`/`employee` share ~74%. The same `checkpoint_flush().is_ok()` expression appears at 16 sites across eight adapters.
- Five server binaries repeat the TLS/auth/exposure startup; the same error string is at `src/bin/dog_server.rs:134`, `entity_server.rs:168`, `fair_play_server.rs:128`, `memory_server.rs:675`, `reminder_server.rs:153`. `signals` exists twice (`memory_server.rs:390`, `fair_play_server.rs:58`).
- `TlsConfig::from_pem_files` (`src/server/serve.rs:3844-3858`) duplicates `read_pem_chain_and_key` (`:3877-3890`) line for line.
- `ServeOptions` has three hand-written full struct literals (`derive(Default)`, `new` at `:3172`, `from_env` at `:3356`); the `Default` drift above is the result.
- **`ConnectionStore` is a 52-method trait** (`src/server/serve.rs:121-858`). 48 mentions of `Unsupported` are default bodies; capability is probed at run time (`strict_commit_supported`, `mvcc_supported`, `range_field`). The default `write_batch` accepts `atomic = true` and applies ops one by one with no rollback (`:566-583`); commit `683a9abc0` had to override it in two Fair Play adapters to stop claiming atomicity.
- Kept-for-symmetry dead field: `MmapAgeStore::path` under `#[allow(dead_code)]` (`src/durability/mmap_store.rs:205-206`).

### 6. Tests for non-trivial logic, including failure and boundary cases; docstrings

Adheres:
- 852 tests. Failure paths are injected, not assumed: `fail_next_append`/`fail_next_sync` (`src/server/changelog.rs:130-135`), the group-commit sync hook (`src/server/journal.rs:546,829`), a crash-safety test that spawns a writer binary (`tests/crash_safety.rs`).
- Wire encoding is pinned by golden vectors (`src/server/protocol.rs:1533`, `tests/fixtures/wire-vectors.txt`).
- Binary tests bind port 0 and read the bound address from the banner (`tests/memory_server_exposure.rs:20-34`).
- Module docs are long and state non-goals. Commit bodies record that each regression test fails with its fix reverted (`40e33158c`, `61096d25c`).

Drifts:
- No test covers the cases found in this audit: token text in `Debug` output (the only `Debug` test checks certificate bytes, `src/server/serve.rs:10598-10611`); `ServeOptions::default()` drain timeout; `FetchSince { after: u64::MAX }`; a journal shorter than its header.
- One test pins a questionable behaviour as correct: a certificate chain path with no key path yields plaintext (`src/server/serve.rs:6105`).
- 475 public items have no doc comment (`missing_docs`): 205 in `src/server/protocol.rs` (enum variants and fields), 46 in `client.rs`, 36 in `audit.rs`. 124 public fallible functions have no `# Errors` section.
- Docs have drifted from code. `src/lib.rs:98` says the protocol version is "31 today"; it is 36 (`src/server/protocol.rs:128`). `src/lib.rs:95` calls `rusty_tls` "this crate's first git dependency"; it is a workspace path dependency. `handle_connection`'s doc says a malformed frame gets one `Response::Err` and that the TLS handshake runs lazily (`src/server/serve.rs:4454-4468`); the code returns silently (`:4793`) and completes the handshake explicitly (`:4668`). `src/production.rs:353` and `src/durability/mmap_store.rs:332` still describe "four `research`-gated backends" that commit `b2a9bb7f5` removed.
- The test helper `struct Server` with `spawn_listening` is copied into four files (`tests/memory_server_exposure.rs`, `memory_server_data_dir_lock.rs`, `memory_server_defaults.rs`, `memory_server_mvcc_integration.rs`), each with `#[allow(dead_code)]` on the fields it does not read.

### 7. Never silently swallow errors

Adheres: the change log poisons itself on any failed append or sync and refuses everything after (`src/server/changelog.rs:479-488`, `:344-349`). Audit and access sinks count dropped events and print one notice (`src/server/audit.rs:324-331`).

Drifts:
- **The server library has no logging at all.** No log crate, and only 2 `eprintln!` in lib code (`src/server/audit.rs:328`, `src/server/access.rs:143`). 50 `map_err(|_| ..)` sites turn an I/O error into a bare wire code and discard the cause (`src/server/changelog.rs:304,424`, `src/server/memory.rs:203,1757`, `src/server/entity.rs:172`). The choice is written down at `src/server/serve.rs:955-963`: "neither error is logged or surfaced to a client". An operator who sees `Storage` has nothing to diagnose it with.
- Journal checkpoint truncation: `let _ = state.journal.truncate();` followed by `state.since_checkpoint = 0` whether or not it succeeded (`src/server/journal.rs:819-820`).
- A failed checkpoint flush becomes "no checkpoint this time" at 16 sites: `checkpoint_flush().is_ok()` (`src/server/memory.rs:714,1978,2044`, `src/server/entity.rs:582`, `src/server/dog.rs:514`, and 11 more).
- `mvcc_flush_now` returns `bool`; a failed read of the insert log becomes a count of `0` that is then written into the history file's bookkeeping (`src/server/memory.rs:1153-1161`; same shape at `entity.rs:921`, `relation.rs:903`).
- `sync_parent` in the change log ignores both the open and the `sync_all` error (`src/server/changelog.rs:148-154`), while the journal uses the engine's fallible `sync_parent_dir` (`src/server/journal.rs:309`).
- Accept errors are skipped with no log and no back-off (`src/server/serve.rs:5723`, `src/server/metrics_http.rs:47`). A framing or decode error ends the connection with no record (`src/server/serve.rs:4793`).
- 40 `let _ =` on `#[must_use]` values in non-test code. Most are socket options and best-effort cleanup; the ones above are not.
- `FileAudit::record` returns without counting when its mutex is poisoned (`src/server/audit.rs:398-400`).

### 8. Minimal dependencies; no secrets logged; input validated at the boundary

Adheres:
- Every dependency carries a reason in `Cargo.toml`. Constant-time comparison is delegated to `subtle` (`src/server/serve.rs:3531-3550`). TLS, HTTP and base64 come from workspace crates behind `optional = true`.
- No key material is committed; TLS tests generate certificates with `rcgen`.
- The binaries refuse a non-loopback bind without both auth and TLS (`src/server/exposure.rs:87-98`), and a non-loopback metrics listener (`:102-108`).
- Backup targets are confined to one directory and installed by rename (`src/server/serve.rs:2844-2877`). Snapshot manifests from a server are checked by a strict portable-name rule before any file is created (`src/server/client.rs:906-929`, commit `7397f5df6`).
- Request sizes are bounded: frame (`src/server/framing.rs:19`), staged ops and bytes (`src/server/protocol.rs:185,200`), chunk length (`src/server/snapshot.rs:149`), tracked peers (`src/server/serve.rs:3621`).

Drifts:
- **`Debug` for `ServeOptions` prints the read-only and read-write tokens** (`src/server/serve.rs:3099-3100`). The same impl redacts `replication_token` (`:3146-3153`), and the struct's doc says tokens "are never logged or echoed back on any path" (`:3009`). `Request` derives `Debug` with `Authenticate { token: String }` (`src/server/protocol.rs:804,845-847`), and `ConnectOptions` derives `Debug` with `token: Option<String>` (`src/server/client.rs:387-389`).
- An exported-but-empty token variable becomes a configured empty token: `std::env::var(..).ok()` (`src/server/serve.rs:3358-3359,3370`) yields `Some("")`, and `check("")` then matches it (`:3531-3550`). By reading. `memory_server` treats empty values as unset for other variables (`src/bin/memory_server.rs:322-325`).
- Half a TLS configuration is silently plaintext: a chain path without a key path falls to `_ => None` (`src/server/serve.rs:3932`), pinned by the test at `:6105`. Only the client-CA-without-chain case is an error (`:3927-3931`).
- The server's own `Backup` name check is the weak rule the client fix replaced: it refuses only `/`, `\`, `.`, `..` and empty (`src/server/serve.rs:2851`), not `:` or device names that `is_plain_file_name` refuses (`src/server/client.rs:906`).
- The snapshot handle, described as "unguessable enough", is `RandomState::new().build_hasher().finish()` (`src/server/snapshot.rs:203-207`). That is a hash-map seed, not a documented source of secret randomness.
- The auth rate limiter fails open when its mutex is poisoned (`src/server/serve.rs:3698-3700`, `:3717-3719`).
- `refuse_busy` starts an unaccounted watchdog thread per refusal that sleeps 6 s (`src/server/serve.rs:5842-5847`). The refusal slot is held by the outer thread only (`:5828-5830`), so a peer that connects past the cap and closes at once frees the slot in milliseconds while the watchdog lives on. Thread count then grows with refusal rate, which the 16-slot pool was meant to prevent (`:5891`). By reading.
- On-disk input is trusted more than wire input. A journal shorter than its 12-byte header is "not a batch journal" and the server will not start (`src/server/journal.rs:318-320`), yet creation writes the header in two calls before the sync (`:304-306`); by reading. Journal and change-log records have a length prefix and no checksum (`src/server/journal.rs:334-342`, `src/server/changelog.rs:202-206`). `MmapAgeStore::open` derives the record count by integer division and never checks the remainder (`src/durability/mmap_store.rs:300-302,577`).
- `rand` is an unconditional dependency (`Cargo.toml`, `[dependencies.rand]`) used only in modules gated on `research` or `cfg(test)` (`src/generator.rs`, `src/bench_support.rs`, `src/generic_spike/order_bench_support.rs`).

### 9. Explicit configuration, no magic globals; modular layout

Adheres: `ServeOptions` is one value built by `with_*` methods; the library reads the environment in only two documented places (`ServeOptions::from_env`, `TlsConfig::from_env`), and `from_env_values` exists so tests never touch the process environment (`src/server/serve.rs:3906-3920`). Feature gating is clean: `client` compiles no server code (`src/server/mod.rs:150-231`).

Drifts:
- `memory_server` makes 21 `std::env::var`/`var_os` calls inline through `main` with three different on/off conventions: presence, so `SERVER_MVCC_ISOLATION=0` turns MVCC **on** (`src/bin/memory_server.rs:459`); strict `0`/`1` (`:350-360`); and exactly `1` (`src/server/exposure.rs:113`). Empty-means-unset is applied to some path variables (`:839`, `:777`) and not to others (`:702`, `:719`, `:425`).
- `SERVER_AUTH_REPLICATION_TOKEN` is read twice, once in `ServeOptions::from_env` and again in `main` (`src/bin/memory_server.rs:710`).
- A process-wide `static IN_FLIGHT` caps metrics connections for every `ServeOptions` in the process (`src/server/metrics_http.rs:27`), unlike the per-options `in_flight` counter.
- `pub use serve::*` (`src/server/mod.rs:231`) makes all 57 `pub` items of `serve.rs` public API, including planner internals such as `intersect_ids` (`src/server/serve.rs:1480`) and `EQ_INTERSECT_RATIO` (`:1460`).
- Production code depends on a module named `test_support` for its temp directory (`src/production.rs:119`, `src/lib.rs:197`), and that directory is never removed (no `Drop` on `ProductionStore`).

---

## B. Lints

What is already enforced for this crate (from `_workspace.md`): `cargo fmt --check`; `cargo clippy -D warnings` at clippy's default set, on five feature sets (`multimodal-db-feature-sets`); `cargo doc -D warnings`; `cargo check` at the declared MSRV; `cargo deny`; `cargo shear`. The crate has **no `[lints]` table** and does not opt in to `[workspace.lints]`. It is excluded from the Windows clippy and test legs.

### Not enforced today

| Lint or check | Hits (lib / bins) | Examples |
|---|---|---|
| `clippy::expect_used` | 46 / 110 (100 of the bin hits are harness prototypes) | `src/production.rs:295`, `src/server/memory.rs:1629`, `src/server/serve.rs:2510` |
| `clippy::unwrap_used` | 4 / 0 | `src/server/mvcc.rs:328`, `:336`, `:494` |
| `clippy::panic` | 0 / 63 (58 in the five server bins) | `src/bin/memory_server.rs:439`, `:450`, `src/bin/dog_server.rs:133` |
| `clippy::unreachable` | 4 / 0 | `src/server/serve.rs:2541`, `src/server/client.rs:1544` |
| `clippy::unwrap_in_result` | 9 / 0 | `src/production.rs:416`, `src/durability/mmap_store.rs:391` |
| `clippy::indexing_slicing` | 96 / 5 | `src/server/changelog.rs:199`, `src/durability/mmap_store.rs:643`, `src/server/journal.rs:318` |
| `clippy::arithmetic_side_effects` | 124 / 22 | `src/server/changelog.rs:414`, `src/server/journal.rs:335`, `src/durability/mmap_store.rs:301` |
| `clippy::cast_possible_truncation` | 30 / 4 | `src/server/changelog.rs:420`, `src/server/serve.rs:1635`, `src/server/memory.rs:1160` |
| `clippy::cast_possible_wrap`, `clippy::cast_sign_loss` | 4 / 6, 2 / 0 | `src/server/serve.rs:2508`, `src/server/sql.rs:642` |
| `clippy::map_err_ignore` | 50 / 0 | `src/server/changelog.rs:304`, `src/server/pem.rs:94`, `src/server/serve.rs:3856` |
| `clippy::let_underscore_must_use` | 27 / 13 | `src/server/journal.rs:819`, `src/server/changelog.rs:151`, `src/server/serve.rs:5829` |
| `clippy::wildcard_enum_match_arm` | 76 / 0 (48 in `client.rs`) | `src/server/serve.rs:2541`, `src/server/serve.rs:3932` |
| `clippy::too_many_lines` (default 100) | 5 / 1 | `src/server/serve.rs:4607` (647), `src/bin/memory_server.rs:417` (339), `src/server/serve.rs:4080` (258) |
| `clippy::too_many_lines` at 60 (needs `clippy.toml`) | 20 / 9 | `src/server/client.rs:1864`, `src/server/sql.rs:326`, `src/server/journal.rs:293` |
| `clippy::cognitive_complexity` | 2 / 1 | `src/server/serve.rs:4607` (109), `src/bin/memory_server.rs:417` (49), `src/server/serve.rs:4080` (35) |
| `clippy::excessive_nesting` at 4 (needs `clippy.toml`) | 104 / 5 | `src/server/client.rs:1543`, `src/durability/mmap_store.rs:582` |
| `clippy::struct_excessive_bools` | 2 / 0 | `src/server/client.rs:511`, `:527` |
| `missing_docs` | 475 / 0 | `src/server/protocol.rs` (205), `src/server/framing.rs:22-26`, `src/server/changelog.rs:92-96` |
| `clippy::missing_errors_doc` | 124 / 0 | `src/server/client.rs` (49), `src/server/serve.rs` (46), `src/server/changelog.rs:160` |
| `clippy::missing_panics_doc` | 6 / 0 | `src/server/mvcc.rs:327`, `:488`, `src/durability/mmap_store.rs:556` |
| `clippy::print_stderr` | 2 / 20 | `src/server/audit.rs:328`, `src/server/access.rs:143` |
| `clippy::print_stdout` | 0 / 39 (all harness) | `src/bin/crash_safety_harness.rs` |
| `clippy::exit` | 0 / 2 | `src/bin/crash_writer.rs:182`, `:198` |
| `unsafe_code` (the workspace table would set `warn`) | 4 / 3 | `src/durability/mmap_store.rs:464`, `:500`, `src/bin/memory_server.rs:404` |
| `clippy::allow_attributes_without_reason` | 4 / 0 | see the allow list below |
| `clippy::match_same_arms` | 8 / 0 | `src/server/serve.rs` (5) |
| `clippy::needless_pass_by_value` | 14 / 0 | `src/durability/mmap_store.rs` (2), `src/server/client.rs` (2) |
| `clippy::must_use_candidate`, `clippy::missing_const_for_fn` | 93, 95 | low value; listed for completeness |
| `clippy::disallowed_methods` for `std::thread::spawn` (needs `clippy.toml`) | 1 / 3, counted by grep | `src/server/serve.rs:5693`, `src/bin/memory_server.rs:406` |
| CI: clippy and tests on Windows | not run | the crate holds Windows-specific reasoning (`src/server/client.rs:896-929`, `src/server/data_lock.rs:17-18`) that no job compiles |
| CI: fuzz or property test of frame and journal decoding | none exists | `src/server/framing.rs:82`, `src/server/journal.rs:293` |

Zero hits, so nothing to enforce against: `clippy::undocumented_unsafe_blocks` (all 7 `unsafe` blocks carry a `SAFETY` comment), `clippy::todo`, `clippy::unimplemented`, `clippy::string_slice`, `clippy::panic_in_result_fn`, `clippy::mem_forget`, `clippy::fn_params_excessive_bools`, `clippy::too_many_arguments`, `clippy::large_enum_variant`.

### Every `allow` in the crate

| Location | Attribute | Switches off an enforced lint? | Reason given |
|---|---|---|---|
| `src/server/serve.rs:731` | `#[allow(clippy::type_complexity)]` | **Yes** (clippy default, `-D warnings` in CI) | No reason given |
| `src/server/changelogged.rs:274` | `#[allow(clippy::type_complexity)]` | **Yes** | No reason given |
| `src/server/changelog.rs:362` | `#[allow(unused_mut)]` | **Yes** (rustc default warning, denied in CI) | No reason given. It exists because a `#[cfg(test)]` branch mutates the binding and the non-test branch does not. |
| `src/durability/mmap_store.rs:205` | `#[allow(dead_code)]` | **Yes** | "kept for symmetry with GenericMmapStore's Self; not read again" |
| `tests/memory_server_exposure.rs:28,31` | `#[allow(dead_code)]` | Yes | "The same helper in every binary test; each reads the fields it needs" |
| `tests/memory_server_data_dir_lock.rs:32,35` | `#[allow(dead_code)]` | Yes | Same copied helper |
| `tests/memory_server_defaults.rs:28,31,34` | `#[allow(dead_code)]` | Yes | Same copied helper |
| `tests/memory_server_mvcc_integration.rs:53,56` | `#[allow(dead_code)]` | Yes | Same copied helper |
| `examples/support/migrate_memory_v1_to_v2_lib.rs:18` | `#![allow(dead_code)]` | Yes | "the CLI and the test each use only part of this file" |
| `examples/support/replica_refresh_lib.rs:679` | `#[allow(unused_imports)]` | Yes | Commit `913991e3d`: a re-export used only by a test target |

No `#[expect(...)]` is used anywhere.

---

## C. General Rust rules

Rules a fluent Rust author applies without being asked, which this code shows were not applied. Stated project-free.

1. Hold a secret in a newtype whose `Debug` and `Display` print a placeholder; never store it as a plain string in a struct that derives or hand-writes `Debug`.
2. When a field is required for some enum variants and meaningless for others, put it inside those variants. Do not validate an `Option` at the boundary and then `expect` it further in.
3. State that is created and discarded together belongs in one `Option<Struct>` or one enum, not in several local variables reset by hand. When flags exclude each other, use an enum.
4. Do not derive `Default` on a type whose constructor sets any field to something other than that field's own default. Implement `Default` by calling the constructor.
5. Choose one response to a poisoned mutex per crate (propagate an error, or recover the guard) and route every lock through one helper that applies it.
6. A default trait method must not accept a parameter that asks for a guarantee the default body cannot deliver. Either make the method required or make the default refuse.
7. When a trait grows optional methods whose defaults say "unsupported", plus methods that report whether they are supported, split it into capability traits that a type either implements or does not.
8. Do not implement `From` for a conversion that performs I/O or can fail. Use a named constructor that returns `Result`, or `TryFrom`.
9. Do not flatten a `Result` to `bool` or to a default value when the caller could propagate it. If the failure is deliberately tolerated, log it at the point it is dropped.
10. Make a callback-taking function generic over the callback's return value, so callers do not pass results out through a `RefCell<Option<T>>` and then `expect` them.
11. An error type that wraps another error implements `source()` and keeps the cause. Do not reuse an unrelated variant, or a bare `String`, to report a different failure.
12. Replace a boolean parameter that selects between two behaviours with a two-variant enum or two methods, so the call site states which behaviour it asked for.
13. Give identifiers and positions their own types. A tuple of two integers, or an alias to a primitive, lets the compiler accept them in the wrong order or the wrong place.
14. Re-export public items by name. A glob re-export of an implementation module makes every `pub` helper in it part of the public API.
15. On the third near-identical copy of a block, extract a generic function or a macro. Copies drift, and a fix must then be made once per copy.
16. Inject test faults through a parameter or a trait, not through `#[cfg(test)]` fields and branches inside production types. Conditional compilation in the middle of an expression needs lint allows and makes the tested code differ from the shipped code.
17. Keep one authoritative table for facts the program needs in more than one place (the version that introduced a variant, the default of an option), and derive documentation, tests and checks from it.

### Evidence

| Rule | File:line or commit | What was seen |
|---|---|---|
| 1 | `src/server/serve.rs:3099-3100` | Hand-written `Debug` prints two token fields; the third is redacted at `:3146-3153` |
| 1 | `src/server/protocol.rs:804,845-847` | `Request` derives `Debug`; `Authenticate { token: String }` |
| 1 | `src/server/client.rs:387-389` | `ConnectOptions` derives `Debug` over `token: Option<String>` |
| 2 | `src/server/protocol.rs:544-547` | `field: Option<FieldRef>`, "`None` only for `Count`" |
| 2 | `src/server/serve.rs:2510,2514,2523,2524,2541` | Four `expect(FIELD_REQUIRED)` and one `unreachable!` after validation |
| 2 | `src/server/client.rs:1544,1667,1939` | `unreachable!` on a parsed-item variant that an earlier branch routed elsewhere |
| 3 | `src/server/serve.rs:4747-4769` | Seven session locals, each commented "cleared with the session" |
| 3 | `src/server/serve.rs:4996-5003,5055-5072,5077-5084,5156-5159,5176-5177` | Five reset sites, each a different subset |
| 3 | `src/server/client.rs:527-533`, `src/server/serve.rs:5039-5045` | Five `bool` options; the server refuses strict commit combined with any other |
| 4 | `src/server/serve.rs:3011,3193,3377,5763` | Derived `Default` gives a zero drain timeout; both constructors give 30 s |
| 5 | `src/production.rs:162,295` | Panic on poison (16 sites) |
| 5 | `src/server/journal.rs:619-621,666` | Error on poison |
| 5 | `src/server/changelog.rs:279`, `src/server/snapshot.rs:84` | Recover the guard on poison (9 such sites in `src/`) |
| 5 | `src/server/serve.rs:3698-3700` | Return `false` on poison (rate limiter off) |
| 5 | `src/server/mvcc.rs:328,336,347,494` | `unwrap()` on poison |
| 5 | `0c18a5788` | "a poisoned index lock is recovered on the scrape and fold paths": two paths patched, the rest left |
| 6 | `src/server/serve.rs:566-583` | Default `write_batch` takes `atomic` and applies ops one at a time, aborting with earlier ops applied |
| 6 | `683a9abc0` | Two adapters override it to refuse atomic batches "before inspecting or applying any operation" |
| 7 | `src/server/serve.rs:121-858` | 52 methods; `strict_commit_supported` (`:591`), `write_batch_strict` (`:596`), `mvcc_supported` (`:792`) |
| 8 | `src/production.rs:432-455` | Two `From` impls create a temp directory and an mmap file and `expect` the result |
| 8 | `src/production.rs:119,301-310` | The temp directory comes from `test_support` and is never removed |
| 9 | `src/server/memory.rs:1148-1162` | `fn mvcc_flush_now(..) -> bool`; a failed log read becomes `0` entries |
| 9 | `src/server/memory.rs:714`, `src/server/dog.rs:514` (16 sites) | `checkpoint_flush().is_ok()` inside a boolean expression |
| 9 | `src/server/changelog.rs:148-154` | `sync_parent` returns `()` and ignores two errors |
| 9 | `src/server/changelog.rs:191,197,198,203` | `try_into().unwrap_or([0; N])` substitutes zeros for a conversion failure |
| 9 | `src/server/journal.rs:819-820` | Truncate error dropped, counter reset anyway |
| 10 | `src/server/journal.rs:692-696` | `apply: impl FnOnce(Turn) -> Result<bool, E>` |
| 10 | `src/server/memory.rs:702-729`, `src/server/entity.rs:570-597`, `src/server/relation.rs:537-562` | Results carried out through `RefCell<Option<Vec<_>>>` then `expect` |
| 11 | `src/server/framing.rs:43`, `src/server/changelog.rs:81` | `impl Error for X {}` on enums that hold `io::Error`/`bincode::Error`; no `source()` |
| 11 | `src/server/serve.rs:3856,3889` | More than one key block reported as `PemError::UnterminatedBlock` |
| 11 | `src/server/serve.rs:3927-3931` | A configuration mistake reported as `io::ErrorKind::NotFound` |
| 11 | `src/bin/memory_server.rs:978`, `src/server/journal.rs:156` | `Result<_, String>`; `Format(String)` covers four distinct failures |
| 12 | `src/server/serve.rs:569,5164` | `write_batch(&ops, true)` |
| 12 | `src/server/serve.rs:2914`, `:2523-2524` | `release: bool`; `reduce_extreme(.., false, ..)` and `(.., true, ..)` |
| 12 | `src/server/changelog.rs:304,331,479` | `append(&ops, true)` / `append(ops, false)` |
| 13 | `src/server/protocol.rs:270,277` | `type RecordId = Uuid`, `type FieldRef = u16` |
| 13 | `src/server/snapshot.rs:25,28,31,36`, `src/server/changelog.rs:92` | `Option<(u64, u64)>` for epoch and sequence beside an existing `LogPosition` struct |
| 13 | `src/server/serve.rs:731`, `src/server/changelogged.rs:274` | `#[allow(clippy::type_complexity)]` on `Result<(Vec<(String, Vec<u8>)>, Option<(u64, u64)>), ErrorCode>` |
| 14 | `src/server/mod.rs:231`, `src/server/serve.rs:1460,1480` | `pub use serve::*` exposes planner constants and helpers |
| 15 | `src/server/memory.rs`, `entity.rs`, `relation.rs` | ~80-88% shared code lines after renaming the domain |
| 15 | `bcbfd419d`, `40e33158c` | One fix edits `memory.rs`, `entity.rs` and `relation.rs` each time |
| 15 | `src/server/serve.rs:3844-3858` vs `:3877-3890` | A helper and its inlined duplicate in the same `impl` |
| 15 | `src/bin/dog_server.rs:134`, `entity_server.rs:168`, `fair_play_server.rs:128`, `memory_server.rs:675`, `reminder_server.rs:153` | Same startup block and message in five binaries |
| 16 | `src/server/changelog.rs:130-135,361-375` | `#[cfg(test)]` fields and a cfg-split block expression needing `#[allow(unused_mut)]` |
| 16 | `src/server/journal.rs:597-598,672-682` | `#[cfg(test)] sync_hook` field in the production struct |
| 17 | `src/server/protocol.rs:1533-1570` | The version that introduced each variant, keyed by string, exists only in a test |
| 17 | `src/server/serve.rs:5007,5289-5300` | The same facts repeated as 32 `if negotiated < N` guards |
| 17 | `src/server/serve.rs:3172-3196,3356-3380` | Two full struct literals for the same defaults |

---

## D. Repo facts

| Fact | Proof |
|---|---|
| Three features: `client` (framing, protocol, client, PEM), `server` (implies `client`), `research` (reference domains and harnesses). `--all-features` hides breakage in the smaller sets, so every `[[test]]`, `[[bin]]`, `[[example]]` and `[[bench]]` needs an explicit `required-features`. | `Cargo.toml` target tables; `5b72bf92d` (five unregistered test targets), `003398f39` ("the lib did not build with `--features server` alone"), `794250162`, `3bdeece40`, `917e51aff`, `f042c7a55`, `913991e3d`, `9279242df` ("The server feature was not in my local clippy set") |
| The `multimodal-db-feature-sets` CI job exists because of the above. Run clippy locally per feature set, not only with `--all-features`. | `5b72bf92d`; `_workspace.md` |
| MSRV is 1.89, fixed by `File::try_lock`. The workspace toolchain is newer, so a std API can be deprecated on the toolchain before its replacement exists at the MSRV; the fix is to hand-write the operation. | `Cargo.toml:5-11`; `7874688ec` (`fetch_update` replaced by a compare-and-swap loop because `try_update` is too new) |
| The crate is excluded from Windows clippy and test legs. | `_workspace.md` ("has never been linted or tested on Windows") |
| The generic store, `codec` and `DurabilityError` live in `rusty_multimodal_db_engine` and are re-exported so old paths resolve. Because of the orphan rule, `DurabilityError::Store` holds a boxed error that this crate downcasts back to `StoreError`. | `src/lib.rs:142`, `src/durability/mod.rs:11-14,41-61`; `414fe987b` |
| Every metric is named `dogserver_*` whatever the domain. | `src/server/metrics.rs:70,244`, `src/server/serve.rs:3419` |
| Comments address decisions by id: ~1,656 `ADR-nnnn` and ~1,845 `XXX-FR-nnn` references in `src/`, against 139 files in `docs/decisions/`. A comment often cannot be understood without the ADR. | `grep -rhoE 'ADR-[0-9]{4}' src \| wc -l` |
| A table's files are found by file-name prefix. A table whose base name is a prefix of another's would pull the other's files into its backup and snapshot. | `src/server/serve.rs:871-888` |
| Opening a generic mmap store folds and clears its pending insert log. Anything that needs the log (MVCC reconstruction) must read it first, and a table must not be opened twice. | `24d880d27`, `9219449ad`, `f8deb1a60`; `src/bin/memory_server.rs:480-488` |
| The journal checkpoint size was "chosen without measurement". | `src/server/journal.rs:71-75` |
| The journal accepts format 2 and 3; a truncate rewrites the header to 3. | `src/server/journal.rs:57-70,484-491`; `40e33158c` |
| The wire protocol is append-only and at version 36. Golden vectors are written to `tests/fixtures/wire-vectors.txt` for foreign clients. A Python reference client is tested and needs `python3` on `PATH`; the test fails without it. | `src/server/protocol.rs:128,1527-1533`; `Cargo.toml` (`server_python_client`, `server_fair_play_integration`) |
| `ServeOptions::default()` is the tests' constructor (123 uses) and means "no auth, no TLS, every peer read-write". The binaries, not the library, refuse to expose it. | `src/server/exposure.rs:4-28` |
| `src/test_support.rs` is compiled into non-test builds on purpose. | `src/lib.rs:194-197`, `src/test_support.rs:4-12` |
| Crates.io dependencies are declared with literal versions (`uuid`, `rand` 0.8, `bincode` 1, `thiserror` 1, `serde`, `memmap2`, `subtle`, `sha2`), not through the workspace's first-party replacements. `Cargo.toml` `repository` names a different repository. | `Cargo.toml:13`, `[dependencies]` |
| ADR numbers collide across parallel branches and get renumbered after the fact. | `8d7433117`, `bcbfd419d` |
| The harness binaries are prototypes: `expect`, `println!` and `process::exit` are their normal style. | `src/bin/crash_writer.rs:182`, `Cargo.toml` (`required-features = ["research"]`) |

---

## E. Non-Rust rules

### Storage and durability

1. After a failed `fsync`, stop writing to that file and fail every writer it covered. Do not retry and treat a later success as covering the earlier bytes.
2. Acknowledge a write only after it is durable, and apply the same rule to every derived structure a later read depends on.
3. After creating or renaming a file, sync its parent directory, and treat a failure there as a failure of the operation.
4. A replayed log entry must not re-run a decision against whatever state replay finds. Record the decision durably before the first effect, and replay the decision.
5. Checksum each log record. A length prefix alone cannot tell a complete record from a torn or zero-filled one.
6. Recovery must accept every state a crash can leave, including a partly written header on a file that was being created.
7. On open, check that a fixed-width file's length is a whole number of records before computing offsets from it.
8. Guard a data directory with a lock the kernel releases when the process dies, taken before any file in it is opened.
9. An open that consumes something (folds and clears a pending log) must return what it consumed. Do not rely on callers reading it first or opening only once.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `src/server/changelog.rs:344-349,382-388` | Applied: a failed sync poisons the log; the comment says a later success "proves nothing" |
| 1 | `src/server/journal.rs:780-784` | Not applied: the journal records `failed_upto` and lets the next batch sync again. The failed entry's bytes stay in the file, so a restart would replay a batch the client was told failed. By reading. |
| 1 | `b6b61d5d7`, `bcbfd419d` | The poisoning was added, then moved into the one function every append passes through |
| 2 | `003398f39` | In-place updates were acknowledged before `msync` |
| 2 | `132e301a5`, `996685ec3` | The MVCC index was written to disk only at compaction; a restart returned stale values |
| 3 | `b9d8cb0b8`, `48d48cb6a` | Parent-directory sync added after every rename install, then after journal and log creation |
| 3 | `src/durability/mmap_store.rs:447-459` vs `:494-495` | `write_fresh` creates the file and does not sync the parent; `write_via_rename` does |
| 3 | `src/server/changelog.rs:148-154` | Parent sync is best-effort and silent |
| 4 | `40e33158c` | Strict replay re-ran the check; a batch refused live was applied after restart, and a partly applied batch was skipped. Fixed with a durable acceptance marker. |
| 5 | `src/server/journal.rs:334-342`, `src/server/changelog.rs:202-206` | Torn-tail detection by length only |
| 6 | `src/server/journal.rs:303-306,318-320` | Header written in two calls; a file under 12 bytes is a hard error on the next open |
| 7 | `src/durability/mmap_store.rs:300-302,577-579` | Record count by integer division; region offsets derived from it with no remainder check |
| 8 | `b9d8cb0b8`, `src/server/data_lock.rs:17-24,75-94` | Two servers on one directory could rewrite each other's mapped files; fixed with `flock` |
| 9 | `24d880d27`, `9219449ad`, `f8deb1a60` | Three separate fixes for "the open cleared the log before the reader got to it" |

### Networking

1. Bound what one peer can make the server hold by bytes as well as by count.
2. Give every multi-step exchange one deadline for the whole exchange. A per-read timeout restarts on every byte.
3. Count every thread or task a peer can cause against the same cap, including helpers such as watchdogs.
4. Before closing a socket that has unread input, shut down the write side and drain, or the reply already sent can be discarded by a reset.
5. A shutdown flag must be checked at each request boundary. Closing the read side does not reach requests already buffered in user space.
6. When `accept` fails, log it and back off before retrying.
7. A client must treat an unexplained close during version negotiation as an error, not as a signal to reconnect with an older protocol.
8. A long-running serve call returns a value that says how it ended, so the caller can decide what is safe to do next.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `61096d25c` | 4,096 staged ops of up to 16 MiB each; a byte budget was added |
| 2 | `3b9c6f716`, `f8deb1a60` | The refusal path got a drain deadline, then a whole-refusal deadline, in two successive fixes |
| 3 | `src/server/serve.rs:5828-5830,5842-5847,5891` | The watchdog thread is spawned outside the 16-slot refusal pool and outlives the slot |
| 4 | `7a92acba4` | Two refusal tests were flaky with `BrokenPipe`/`ConnectionReset` until the close became a FIN after a drain |
| 5 | `6017a748e` | A pipelined request ran after shutdown; the flag is now checked before reading and before acting |
| 6 | `src/server/serve.rs:5721-5724`, `src/server/metrics_http.rs:45-48` | `Err(_) => continue` with no log and no delay |
| 7 | `996685ec3` | The client re-dialled at protocol 1 when a full server closed under its `Hello` |
| 8 | `6017a748e` | `serve` returned `()`; it now returns a must-use outcome and the binary marks logs clean only on a full drain |

### Security

1. A listener reachable from other hosts refuses to start without authentication and encryption. A warning in a banner is not a control.
2. A security setting that is half configured is a startup error, never a silent fall back to the insecure mode.
3. An empty credential is not a credential. Reject it when configuration is read.
4. Validate a peer-supplied file name with one strict rule shared by every component that joins it to a path.
5. Generate handles that act as capabilities from the operating system's secure random source.
6. A control that limits attackers fails closed when its own state is unusable.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `7d02359a6`, `src/server/exposure.rs:4-12,87-98` | A server on `0.0.0.0` with nothing set served every peer read-write "and said so only in a banner" |
| 1 | `3b9c6f716` | The same refusal was then added for the metrics listener |
| 2 | `src/server/serve.rs:3922-3933,6105` | Chain path without key path gives `None` (plaintext); a test asserts it |
| 3 | `src/server/serve.rs:3358-3359,3370,3531-3550` | `env::var(..).ok()` keeps `""`; `check` compares it as a token. By reading. |
| 4 | `7397f5df6`, `src/server/client.rs:906-929` | `C:escape` passed the clients' check; one strict rule now covers both clients |
| 4 | `src/server/serve.rs:2851` | The server's `Backup` name still uses the earlier, weaker rule |
| 5 | `src/server/snapshot.rs:203-207` | Handle taken from a `RandomState` hasher |
| 6 | `src/server/serve.rs:3698-3700,3717-3719` | A poisoned rate-limit table reports "not throttled" |

### Concurrency

1. Read a value and register interest in it inside one critical section. Two sections leave a gap in which the value can be reclaimed.
2. Keep a running count for anything a metrics scrape reads. Do not walk a large structure under the lock that writers need.
3. Wake the one waiter whose turn it is. Waking all of them costs time quadratic in the queue.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `996685ec3` | Snapshot begin read `last_committed` and registered in two steps; history could be reclaimed below a live snapshot |
| 2 | `48d48cb6a` | A metrics scrape walked every version chain under the index lock |
| 3 | `src/server/journal.rs:789-800` | Per-sequence `park`/`unpark` replaced a condvar; the comment records ~3k against ~30k batches/s at 32 connections |

### Testing

1. A test binds port 0 and reads the bound address back. It never picks a free port first.
2. Code that reads the environment is a thin wrapper over a pure function that takes the values; tests call the pure function.
3. A regression test is only evidence if it fails with the fix reverted. Check that and say so.
4. A test that needs an external tool fails when the tool is missing. A skipped test reports nothing.
5. Increment a counter at the point the outcome is decided, not where the request arrives.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `3d911dc08`, `tests/memory_server_exposure.rs:20-24` | `free_port()` removed; the server prints its bound address and tests read it |
| 2 | `src/server/serve.rs:3352-3355,3906-3920` | `from_env` delegates to `from_env_values`; the doc explains that parallel tests would race on real variables |
| 3 | `40e33158c`, `61096d25c`, `9219449ad` | Commit bodies state "Each fails when its fix is reverted" |
| 4 | `Cargo.toml` (`server_python_client`) | "fails loudly without one — a skipped test is not a test" |
| 5 | `3b9c6f716` | A "rows clamped" counter was incremented for requests later refused as unauthenticated |

### Configuration and CLI

1. Parse every on/off setting with one function and one convention. Presence-means-on makes `NAME=0` enable the feature.
2. Decide once whether an empty value means unset, and apply it to every variable.
3. Read each setting in one place and pass the value on.

| Rule | Evidence | What was seen |
|---|---|---|
| 1 | `src/bin/memory_server.rs:459` vs `:350-360` vs `src/server/exposure.rs:113` | Presence, strict `0`/`1`, and exactly-`1` conventions side by side |
| 1 | `48d48cb6a` | One variable was converted from presence to a `0`/`1` switch after the fact |
| 2 | `48d48cb6a`, `0c18a5788`; `src/bin/memory_server.rs:839,777` vs `:702,719,425` | "empty env values are unset" applied to some path variables and not others |
| 3 | `src/server/serve.rs:3370`, `src/bin/memory_server.rs:710,714` | One token variable read three times |

# Rust adherence audit: summary

Commit audited: `f80ee7fc5572de65928be3f7df019ae1f056c7e3`. Reports merged here:
[`_workspace.md`](_workspace.md), [`rusty_mill_board.md`](rusty_mill_board.md),
[`rusty_mux.md`](rusty_mux.md), [`rusty_multimodal_db.md`](rusty_multimodal_db.md),
[`rusty_bullet.md`](rusty_bullet.md).

Crate short names used below: **board** = `rusty_mill_board` (+ runner), **mux** = the 11
`rusty_mux_*` packages, **mmdb** = `rusty_multimodal_db`, **bullet** = the 12 `rb_*` packages plus
`rb_tape_bot`. Rule references such as "board C5" point at the numbered rule in that crate's report,
where the evidence table is.

## How far to trust this

- Each crate report was written by a separate auditor that read only its own crate, its history and
  `_workspace.md`. No expert prompts, prior reviews or audit reports were read.
- Lint counts are real clippy 1.98.1 runs on non-test code (lib and bin targets), on Linux. Not
  linted: `rusty_mux_gui` (grep only) and all `cfg(windows)` code.
- board was read in full. mux, mmdb and bullet were read in full for their storage, network and
  protocol modules and sampled elsewhere; each report's header lists what was sampled. Test bodies
  were counted, not reviewed.
- Three mmdb findings are marked "by reading" in its report (not reproduced).
- I spot-checked nine citations across the four reports against the source; all nine held. The rest
  were verified only by the auditor that wrote them.
- The recommended lint block below has **not** been compiled against the workspace.

## Section C rules, merged

"Workspace explains" asks whether a convention found in `_workspace.md` accounts for the rule
recurring: **yes**, **no**, or **partly**.

| # | Rule | Crates | Workspace explains |
|---|---|---|---|
| 1 | Keep the original error as `source()` and add what was being done and to what. Do not flatten to `String` at the first boundary, reuse an unrelated variant because its payload is a string, or write a blanket `From` that maps every source error to one variant. Public library functions return a typed error. | bullet (C6, C7), mmdb (C11), mux (C10, C11); board shows it in section A (83 `String`-error signatures) | partly: a shared error crate exists in `foundation` and none of the four uses it, so each invented its own style |
| 2 | Carry an error's kind as an enum value from where it is raised to where it is handled. Never render it to text and match on a prefix or phrase. | mux (C1) | no |
| 3 | When an operation has more than two outcomes, return an enum that names them. Do not carry domain outcomes in a `bool` inside `Result` or in the `ErrorKind` of an I/O error no I/O produced. | bullet (C4) | no |
| 4 | Map errors to transport status codes in one exhaustive `match` with no wildcard arm, in one function every client and test can reach. | board (C16) | no |
| 5 | Do not turn a parse failure or an error into a null, a default or a `bool` and carry on. Return it where it happened; if it is deliberately tolerated, log it at the point it is dropped. | board (C17), mmdb (C9) | no |
| 6 | In a decoder, return an error that says which check failed, even when the caller will only drop the input. An `Option` leaves nothing to log or count. | mux (C14) | no |
| 7 | Hold one entity's lifecycle in a single enum-valued field. Do not spread it across booleans, an `Option` plus flags, optional fields that must agree, or local variables reset by hand. | board (C7), bullet (C12), mmdb (C3), mux (C2) | no |
| 8 | When several collections are indexed by the same entity, store one collection of a per-entity struct. Parallel vectors or maps must each be remembered on every insert and removal. | bullet (C1), mux (C2) | no |
| 9 | When a field is required for some variants or modes and meaningless for others, put it inside those variants or type it `Option`. Do not validate an `Option` at the boundary and `expect` it later, and do not fill an uncomputed field with zero. | bullet (C8), mmdb (C2) | no |
| 10 | When a reduced read fills only part of a record, give it its own type. | board (C5) | no |
| 11 | When a struct's fields are checked by a constructor, keep them private. Do not make one field public when private fields must stay in step with it. | board (C6), bullet (C2) | no |
| 12 | Give one type one policy for an out-of-range index: every accessor returns `Option`/`Result`, or the index is checked once where it enters. | bullet (C3) | no |
| 13 | A public constructor that can reject its arguments returns `Result`. Keep `assert!` for conditions only a bug in the same module can break. | bullet (C5) | no |
| 14 | Check a precondition inside the function that depends on it, once. A check in one of several callers, or only in a doc comment, is not a check. | bullet (C9) | no |
| 15 | Parse a closed set of strings into an enum once, with an explicit unknown case, and match on the enum exhaustively afterwards. Do not compare string literals or match on the name again after a table lookup. | board (C8), mux (C4) | no |
| 16 | An unknown verb, mode, kind or key is an error. Do not write a fallback arm that turns it into a valid default, and reject unknown keys when every field of a deserialised struct has a default. | board (C9), bullet (C10) | no |
| 17 | Give identifiers, positions, sessions and same-typed arguments with different meanings their own types. Do not use bare integers of varying width, tuples of integers, or the address of a shared handle as identity. | bullet (C11), mmdb (C13), mux (C8, C9) | partly: a shared id crate (`rusty_uuid`) is used for values, but nothing supplies typed ids |
| 18 | Give every type that carries a secret a redacting `Debug`, ideally by holding the secret in a newtype that prints a placeholder. Do it for every such type, not only the first. | board (C10), mmdb (C1); mux shows it in sections A and E (E9) | partly: no scoped crate depends on a shared secret type, so each redacts by hand and misses some. Whether `foundation` already offers one was not checked |
| 19 | Choose one response to a poisoned mutex per crate, route every lock through one helper, and do not panic while holding a lock whose poisoning is reported as permanent. | board (C3, C4), mmdb (C5) | no |
| 20 | When a "begin" must be matched by an "end", or an OS resource must be released, return an owning guard whose `Drop` does it, and hand it to its long-term owner before anything that can fail. | board (C2), mux (C6) | no |
| 21 | When implementing a trait for a wrapper by forwarding, forward every method including defaulted ones, and test each through the wrapper. | board (C1) | no |
| 22 | A default trait method must not accept a parameter that asks for a guarantee the default body cannot deliver. Make it required or make the default refuse. | mmdb (C6) | no |
| 23 | When a trait grows optional methods whose defaults say "unsupported", split it into capability traits. | mmdb (C7) | no |
| 24 | Do not derive `Default` on a type whose constructor sets any field to something else. Implement `Default` by calling the constructor. | mmdb (C4) | no |
| 25 | Do not implement `From` for a conversion that performs I/O or can fail. | mmdb (C8) | no |
| 26 | Replace a boolean parameter that selects between two behaviours with an enum or two methods. | mmdb (C12) | no |
| 27 | A type holds only values that are legal where it is used: give each direction of a channel its own message enum, and keep decode-only "unknown" variants out of the type the encoder accepts. | bullet (C13), mux (C3) | no |
| 28 | Keep the `JoinHandle` of a long-lived worker, and handle the `Err` of every `join`. | board (C11, C12) | no |
| 29 | Do not hold a lock across network or disk I/O: snapshot and take an epoch under the lock, do the I/O, re-lock and retry if the epoch moved. Where check and commit must agree, hold one lock from check through commit. | board (C14, C15) | no |
| 30 | When a shared map keeps "the latest version seen", merge with `max`, not overwrite. | board (C13) | no |
| 31 | Never make a blocking system call on the thread that runs a single-threaded executor, `Drop` impls included. | mux (C12) | partly: the runtime is a first-party crate with its own API, so habits from the common third-party runtime do not transfer |
| 32 | A future does nothing until polled. A step that must happen even if the task is dropped unpolled belongs in a plain function before the future is created. | mux (C5) | no |
| 33 | A function that takes ownership of something the caller could still own, such as a raw descriptor, is `unsafe fn` with written preconditions. | mux (C7) | no |
| 34 | Move shared logic into the library before writing a second copy for a second binary, and extract on the third near-identical block. | bullet (C14), mmdb (C15) | no: both cases are inside one family, where the layer rule does not get in the way |
| 35 | Keep `main.rs` to argument parsing and wiring, and inject test faults, time and id generation through parameters or traits used everywhere, not through `#[cfg(test)]` fields in production types or a direct clock read in one adapter. | board (C20), mmdb (C16), mux (C13) | partly: no shared test-utility crate is in use, so each crate built its own seams |
| 36 | Keep one authoritative definition for a fact or constant needed in more than one place and derive the rest. | bullet (C17), mmdb (C17) | no |
| 37 | When a lint objects to a signature, change the signature. If it is silenced, write the reason in the attribute. | bullet (C15) | partly: nothing requires a reason on `allow` and there is no `clippy.toml` to allow test-only panics, so bare allows are the path of least resistance |
| 38 | Keep project status ("not built yet", "three modes", "v0") out of doc comments and manifest descriptions. | bullet (C16) | no |
| 39 | Re-export public items by name; a glob re-export makes every `pub` helper public API. | mmdb (C14) | no |
| 40 | Make a callback-taking function generic over the callback's return value. | mmdb (C10) | no |
| 41 | Substitute placeholders in one pass over a template; chained `replace` re-scans inserted text. | board (C18) | no |
| 42 | Code under `#[cfg(...)]` needs its imports under the same `cfg`, and must be compiled and linted on every target it claims. | board (C19) | yes: a PR runs Linux tests and Linux plus Windows clippy only; macOS and Windows tests first run in the merge queue |
| 43 | Keep string-offset arithmetic on the string the offsets came from. | board (C21) | no |

Rules shown by all four crates: 7 (lifecycle as one enum) and, counting section A evidence, 1
(typed errors with a source). Rules shown by three: 17, 18, 35.

## Section E rules, merged, by topic

### Storage and durability

| Rule | Crates | Workspace explains |
|---|---|---|
| After a failed `fsync`, stop writing to that file and fail every writer it covered. | mmdb | no |
| Acknowledge a write only after it, and every derived structure a later read depends on, is durable. Do not acknowledge what was not recorded. | mmdb, mux | no |
| After creating or renaming a file, sync its parent directory. A journal used for undo must be durable, directory entry included, before the first change. | mmdb, board | partly: an atomic-file crate exists in `foundation`; only one scoped crate uses it |
| Before an irreversible effect, durably record the intent keyed so a retry finds it. On replay, replay the recorded decision; do not re-decide against whatever state replay finds. | board, mmdb | no |
| Validate every part of a compound operation before the first side effect, and report a split write as its own outcome. | board, mux, bullet | no |
| Commit a state change and its log entry as one durable record, and assign the log id inside the commit lock. | board | no |
| Allocate identifiers that must be unique through the store with create-if-absent, not a process-local `max + 1`. | board | no |
| Checksum each log record; a length prefix cannot tell complete from torn. | mmdb | no |
| Recovery must accept every state a crash can leave, including a partly written header. | mmdb | no |
| On open, check a fixed-width file's length is a whole number of records. | mmdb | no |
| Guard a data directory with a lock the kernel releases on process death, taken before any file is opened. | mmdb | no |
| An open that consumes something returns what it consumed. | mmdb | no |
| When reader and writer can be different versions, keep and write back unknown fields. | board | no |
| Treat an unparseable stored record the same on every read path. | board | no |
| Write outputs under a temporary name beside the destination and rename when complete. First prove the output is not an input under another name. Never overwrite a file the user owns without a backup. | bullet, mux | partly: same atomic-file crate, unused here |
| A pattern that selects output files must not match their sidecars. | bullet | no |
| A path placed inside JSON or TOML needs that format's escaping. | bullet | no |

### Networking and resource bounds

| Rule | Crates | Workspace explains |
|---|---|---|
| Bound everything one peer can make the server hold, by count and by bytes: queues, clients, helper threads, closed-but-undrained resources, in-memory logs, list responses. Bound a value before it fans out. | board, bullet, mmdb, mux | no |
| Give every exchange, outbound call and subprocess one deadline that covers connect, read and write. A per-read timeout restarts on every byte. | board, bullet, mmdb, mux | no |
| No single bad packet or `accept` error may end a server: log, back off, and sort errors into skip, pause and fatal. | mmdb, mux | no |
| Shut down in order: close admission, stop the listener, wait for in-flight work under one deadline. Check the flag at each request boundary, and return how the serve call ended. | board, mmdb | no |
| Handle everything already received before acting on a close; drain before closing a socket with unread input; keep reading from a peer while waiting on something else. | bullet, mmdb | no |
| When a reply would exceed its limit, refuse it whole or page whole records. Never return a silent prefix. | board, mux | no |
| Do not use a socket receive timeout as a polling interval, do not map an unexpected error code to "timeout", and never leave a partial frame on a nonblocking socket. | bullet | no |
| A paging loop fails if the cursor does not advance or a continuing page is empty. Check how the server reads repeated query parameters. | board | no |
| Decide which connection is the one awaited by what it sends, not by arriving first. | bullet | no |
| Name the failing call in every I/O error. | bullet | no |
| Refuse a request that grows another party's queue once that queue is half full. | mux | no |
| Do not reuse a base URL configured for a different host. | board | no |

### Protocol and API design

| Rule | Crates | Workspace explains |
|---|---|---|
| Decode required fields strictly and pin encoders to bytes from the reference implementation. A message carrying state that cannot be decoded is fatal. | bullet | no |
| Do not trust the documented meaning of an external system's field until it has been observed live. | bullet | no |
| Check negotiated capabilities before sending or waiting for anything a peer without them would ignore. Treat an unexplained close during negotiation as an error, not a cue to downgrade. | mmdb, mux | no |
| An argument or option value of the wrong type, or one that does not parse, is an error. Do not drop it or fall back to the default. | board, bullet | no |
| When a refresh or reload fails, keep the last good data and mark it stale. | board, mux | no |
| If a request has no success reply, provide an ordered no-op the client can send after it. | mux | no |
| When rotating a token, keep accepting the last one the client proved it held until it proves the new one. | mux | no |
| Never reuse a retired message type, field id or capability bit. | mux | no |
| Make a default response small and let the caller ask for the full form. | board | no |
| A renderer of untyped pass-through data skips a malformed item instead of aborting the list. | board | no |
| An error message states only what was established. | mux | no |
| Restore the terminal on every exit path. | mux | no |

### Security

| Rule | Crates | Workspace explains |
|---|---|---|
| A listener reachable from other hosts refuses to start without authentication (and encryption). A header check or a banner warning is not a control. | board, mmdb | no |
| A half-configured security setting, an empty credential, or a retired configuration key that changes where data lives is a startup error. Controls fail closed. | board, mmdb | no |
| Resolve authority from the credential or transport-verified identity presented, by a stable identifier, never from a supplied name. Bind session tokens to that identity. | board, mux | no |
| Give child processes an allowlisted environment, fresh standard streams and close-on-exec descriptors. | board, mux | partly: a sandbox crate exists in `libs`; only board's runner uses it |
| Check a file's type, owner and mode on the opened handle; set permissions at creation; refuse symbolic links in privileged paths; create scratch directories exclusively under a fresh name. | board, mux | no |
| Validate peer-supplied file names and links by structure, with one strict rule shared by every component. | board, mmdb | no |
| Do not log a peer's message whole; escape control characters from peers before printing; hide other people's names in diagnostics unless asked. | bullet, mux | no |
| Accept secrets from the environment or an owner-only file, not command-line arguments, for every secret. | board | no |
| Signal a process through a handle that cannot be reused, never a stored pid. | mux | no |
| Generate capability handles from the operating system's secure random source. | mmdb | partly: a first-party random crate exists and was not used at that site |

### Concurrency

| Rule | Crates | Workspace explains |
|---|---|---|
| Check and act in one critical section: run the guard inside the compare-and-swap, read-and-register together, and allow no suspension point between a check and recording its result. | board, mmdb, mux | no |
| Make every write to a shared record a compare-and-swap on the version read. | board | no |
| Match a late result against the connection or generation that asked for it. | mux | no |
| Do not use a writer's timestamp as a read cursor across writers, and never advance a high-water mark past a skipped item. | board | no |
| Tear down in dependency order. | mux | no |
| Keep a running count for anything a metrics scrape reads; wake the one waiter whose turn it is. | mmdb | no |

### Numerics and determinism

| Rule | Crates | Workspace explains |
|---|---|---|
| Convert the unit of every constant ported from another code base. | bullet | no |
| Detect time moving from an integer counter, not floating-point seconds, and decide whether each counter wraps or saturates. Drive playback from the authoritative counter, not a message count. | bullet | no |
| Compare two recordings by timestamp, not by row. | bullet | no |
| Test determinism directly: two instances, and a reused instance after reset, must match bit for bit. | bullet | no |
| A parameter fitted on one window is a candidate until it holds on data it was not fitted on. | bullet | no |
| Reject or clamp non-finite and out-of-range numbers at every boundary, files included. | bullet | no |

### Testing

| Rule | Crates | Workspace explains |
|---|---|---|
| Wait for the observable condition with a bounded poll. Never sleep for a fixed time. | board, mux | yes: nextest `retries = 2` lets a test that fails once in three pass, so the sleep is never forced out |
| A regression test is evidence only if it fails with the fix removed. Check that and say so. | board, mmdb, mux | no |
| Give every connection a test opens a timeout, hold the processes a test starts by handle, and do not let a stand-in peer close on an event that races the behaviour under test. | bullet, mux | partly: no shared test-utility crate is in use |
| Put environment reads in a thin wrapper over a pure function, and test the pure function. | mmdb | partly: no shared configuration crate is in use |
| Bind port 0 and read the address back. | mmdb | no |
| Tests that depend on creation order need a clock that advances on every read. | board | no |
| A test that needs an external tool fails when the tool is missing. | mmdb | no |
| Check against the real system at least once and keep what it sent as anonymised fixtures. | bullet | no |
| Look for a test's marker only where the behaviour under test would put it; retry interrupted reads in test clients. | mux | no |
| Increment a counter where the outcome is decided, not where the request arrives. | mmdb | no |

### Configuration and CLI

| Rule | Crates | Workspace explains |
|---|---|---|
| Parse every on/off setting with one function and one convention, decide once whether empty means unset, and read each setting in one place. | mmdb; bullet and mux show the same drift in section B (environment reads outside `main`: 5 and 28 sites) | partly: a shared configuration crate exists and none of the four uses it |
| In a tool that stops something the user is running, do every step that can fail first. | bullet | no |
| When editing a file the user owns, change only the bytes you wrote. | mux | no |

## Section B lints, merged

Hits are non-test code. "enf" means the crate already enforces it. mux counts exclude
`rusty_mux_gui`; mmdb is lib / bins. "~" is approximate.

| Lint or check | board | mux | mmdb | bullet | Call |
|---|---|---|---|---|---|
| `clippy::unwrap_used` | enf | 0 | 4 / 0 | enf (`rb_tape_bot`: 0) | adopt now |
| `clippy::expect_used` | 1 | 0 | 46 / 110 | enf (`rb_tape_bot`: 13) | adopt now; mmdb needs a fix pass |
| `clippy::panic` | 0 | 0 | 0 / 63 | enf (`rb_tape_bot`: 0) | adopt now; mmdb's server binaries need `main -> Result` |
| `clippy::todo`, `unimplemented`, `dbg_macro`, `unwrap_in_result`, `panic_in_result_fn` | 0 | 0 | `unwrap_in_result` 9 | 0 | adopt now |
| `unsafe_code` (rustc) | forbid | 9 (ctl 2, server 7) | 4 / 3 | deny or forbid (`rb_tape_bot`: none, 0 hits) | keep `warn` at workspace; `forbid` per crate where zero |
| `clippy::undocumented_unsafe_blocks` | n/a | 0 | 0 | n/a | adopt now (already in the workspace table; these crates do not inherit it) |
| `clippy::allow_attributes_without_reason` | 4 | 5 | 4 | 4 non-test (62 with tests) | adopt now |
| `clippy::map_err_ignore` | 11 | 36 | 50 | 13 | adopt, fix pass |
| `clippy::let_underscore_must_use` | 3 | 63 | 27 / 13 | 3 | adopt, fix pass (each survivor gets `#[expect]` with a reason) |
| `clippy::string_slice` | 11 | 12 | 0 | 1 | adopt, fix pass |
| `clippy::wildcard_enum_match_arm` | 3 | 46 | 76 | 11 | adopt, fix pass |
| `clippy::missing_errors_doc` | 24 | 118 | 124 | 62 | adopt with `missing_docs` |
| `missing_docs` (rustc) | enf | 520 | 475 | 497 | adopt per crate as each is documented; too large for one step |
| `clippy::indexing_slicing` | 31 | 76 | 96 / 5 | 283 | stage 2 |
| `clippy::cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap` | 0, 0, 1 | 35, -, - | 34, 2, 10 | 25, 5, 3 | stage 2 |
| `clippy::arithmetic_side_effects` | 61 | 249 | 146 | 444 | do not adopt workspace-wide; use it as a review list for wire, journal and frame-counter code |
| `clippy::too_many_lines` (100) | 2 | 9 | 5 / 1 | 11 | adopt with `#[expect]` on today's offenders |
| `clippy::cognitive_complexity` (25) | 0 | 0 | 2 / 1 | 1 | adopt |
| `clippy::excessive_nesting` (needs `clippy.toml`) | 109 / 28 / 3 at 3 / 4 / 5 | not counted | 109 at 4 | ~53 functions past 3 | adopt at 5, ratchet to 3 |
| `clippy::struct_excessive_bools`, `fn_params_excessive_bools` | 0 | 1 | 2 | 4, 1 | adopt |
| `clippy::partial_pub_fields` | 2 | 1 | - | 5 | adopt |
| `unreachable_pub` (rustc) | 0 | - | - | 27 | adopt |
| `clippy::clone_on_ref_ptr` | 8 | 42 | - | - | optional |
| `clippy::match_bool` | 4 | 51 | - | - | optional |
| `clippy::needless_pass_by_value` | 12 | 8 | 14 | - | optional |
| `clippy::print_stdout`, `print_stderr` in library targets | 0 | 75 `stderr` (mostly binaries) | 2 | 1 | per library crate, not workspace: binaries print on purpose |
| `clippy::must_use_candidate`, `missing_const_for_fn`, `doc_markdown` | 61, -, 66 | 133, 94, 54 | 93, 95, - | 220, -, 306 | skip: high volume, low value |
| `clippy::disallowed_methods` for `std::env::var`/`var_os` (needs `clippy.toml`) | - | 28 sites | - | 5 | adopt after one config seam per binary exists |
| `clippy::debug_assert_with_mut_call` (nursery) | - | 0 now, 1 in history | - | - | adopt |
| `clippy.toml` `allow-{unwrap,expect,panic,indexing-slicing}-in-tests` | - | - | - | removes 54 of 62 allows | adopt |
| CI: `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings` | clean, not run | not run | enforced | not run | add for all |
| CI: clippy and tests on Windows for mmdb | | | excluded | | remove the exclusion once a run is green |
| CI: fuzz or property tests of frame, journal and datagram decoders | | none | none | | add |
| CI: `mux_egress_audit.sh` | | exists, never called | | | wire it or delete it |
| CI: report of tests that passed only on retry | all | all | all | all | add; nextest's JUnit already records it |

`#[allow]` attributes that switch off an enforced lint: every one of the four crates has them and
almost none states a reason (bullet 62, board 4, mmdb 4 in library code, mux 5). All but a handful
are test-only `unwrap_used`/`expect_used`/`panic` allows, which a `clippy.toml` removes. The
non-test ones are in bullet (`collision.rs:1382`, `:1504`, `:1709`, `solver.rs:748`) and mux
(`server/src/state.rs:1804`, `too_many_arguments`).

### Recommended `[workspace.lints]` block

Three things to know before adopting it:

- A crate gets this table only with `[lints] workspace = true`, and Cargo does not let a crate
  inherit the table and also add its own entries. board and the `rb_*` crates would replace their
  tables; a crate-specific exception becomes a `#![expect(lint, reason = "...")]` in `lib.rs`.
- Eleven other crates already inherit the workspace table. Growing it changes their CI. Either fix
  them in the same PR or stage the opt-in.
- CI denies warnings and lints `--all-targets`, so tests are linted too. The `clippy.toml` below
  covers `#[test]` functions and `#[cfg(test)]` modules; check whether helper functions in
  `tests/*.rs` are covered before deleting the per-file allows.

```toml
# Stage 1: zero or single-digit hits in board, mux and bullet; mmdb needs a fix pass first.
[workspace.lints.rust]
unsafe_code = "warn"            # existing; crates with none keep `forbid` via #![forbid(unsafe_code)]
unreachable_pub = "warn"
# missing_docs = "warn"         # turn on per crate as each is documented (~1,500 hits today)

[workspace.lints.clippy]
undocumented_unsafe_blocks = "deny"   # existing
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
todo = "warn"
unimplemented = "warn"
dbg_macro = "warn"
unwrap_in_result = "warn"
panic_in_result_fn = "warn"
allow_attributes_without_reason = "warn"
map_err_ignore = "warn"
let_underscore_must_use = "warn"
string_slice = "warn"
wildcard_enum_match_arm = "warn"
struct_excessive_bools = "warn"
fn_params_excessive_bools = "warn"
partial_pub_fields = "warn"
debug_assert_with_mut_call = "warn"
too_many_lines = "warn"
cognitive_complexity = "warn"
excessive_nesting = "warn"

# Stage 2: after the indexing and cast sites have been reviewed (~500 and ~150 hits).
# indexing_slicing = "warn"
# cast_possible_truncation = "warn"
# cast_sign_loss = "warn"
# cast_possible_wrap = "warn"
# missing_errors_doc = "warn"
```

```toml
# clippy.toml (new, repository root)
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-indexing-slicing-in-tests = true
too-many-lines-threshold = 100
cognitive-complexity-threshold = 25
excessive-nesting-threshold = 5      # ratchet 5 -> 4 -> 3; the preference is 3
```

Cheapest first step: mux can inherit stage 1 minus `map_err_ignore`, `let_underscore_must_use`,
`string_slice`, `wildcard_enum_match_arm` and the size lints with **no code change**, which locks
in a zero-panic record that nothing protects today.

## Five findings per crate that would move the most

### rusty_mill_board

1. **Parse failures become null or vanish.** A stored document that fails to parse is dropped from
   the listing path without a log (`src/tick.rs:778`) while the single-card path reports it; a
   malformed request body becomes `Null` (`src/web.rs:291`, `runner/src/http_board.rs:58`); a
   panicked stage thread is discarded (`runner/src/runner.rs:214`); a dead sync thread still reports
   as running (`src/syncer.rs:51-79`).
2. **`Card` permits illegal states and the store returns half-filled cards.** 29 public fields, end
   state spread over four of them, and `Store::summaries` returns full `Card`s guarded by a comment
   (`src/store.rs:28-37`) although a summary type exists.
3. **The one `expect` sits under a lock whose poisoning is permanent** (`src/tick.rs:984`). If the
   manual begin/end pairing breaks, every later store call fails until restart. A drop guard plus
   `clippy::expect_used` closes it.
4. **`ops()` is 425 lines of 35 closures, each repeating its own authorisation checks**
   (`src/api.rs:712`). Two fix commits (`9fafef5b3`, `57114de55`) were checks missed in single
   closures.
5. **Boundary gaps on a service that takes agent input:** no length bounds on text fields, an
   unchecked `limit` on `board_events` (`src/api.rs:872`), an event log that never trims
   (`src/tick.rs:578-605`), a subprocess with no timeout (`src/bbp_cli.rs:32-35`), a token under
   derived `Debug` (`src/settings.rs:82-95`), and three of four tokens accepted on the command line.

### rusty_mux

1. **Error kinds cross the wire as string prefixes and are parsed back**, down to matching English
   prose (`gui/src/lease.rs:142`, `:166`, `:171`; raised at `core/src/error.rs:165`). The wire's
   code enum has two variants.
2. **Server lifecycle state is spread across six parallel maps and booleans**
   (`server/src/state.rs:402-420`). Six fix commits landed in exactly this bookkeeping.
3. **The zero-panic, documented-unsafe record is unenforced.** None of the 11 manifests has a
   `[lints]` table; the stage 1 core can be switched on today with no code change.
4. **Wildcard arms defeat exhaustiveness where the protocol grows.** In `receives()`
   (`server/src/state.rs:2589`) a new event kind without an arm is sent to every client, including
   ones that never negotiated it.
5. **Two files carry too much and tests lean on sleeps.** `tui/src/main.rs` is 4,975 lines,
   `server/src/state.rs` 4,046 with a 275-line `apply`; 87 `sleep` calls in tests, masked by
   nextest's retries. Also: a pre-handshake frame of up to 16 MiB is printed with `{:?}`
   (`server/src/conn.rs:51`).

### rusty_multimodal_db

1. **Tokens are printable through `Debug`.** The hand-written impl prints both access tokens
   (`src/server/serve.rs:3099-3100`) while redacting a third; two more types derive `Debug` over a
   plain-string token (`protocol.rs:804,846`, `client.rs:387-389`).
2. **Errors are dropped on the durability path.** A failed journal truncate is discarded and the
   checkpoint counter reset anyway (`src/server/journal.rs:819-820`); the journal retries after a
   failed `fsync` where the change log correctly fail-stops; 50 `map_err(|_| …)` sites record
   nothing, and the server library has no logging.
3. **Illegal states at the core of the server.** A 647-line connection handler with session state
   in ten locals reset by hand; a 52-method trait whose default `write_batch` accepts
   `atomic = true` without rollback (patched in two adapters by `683a9abc0`); a derived `Default`
   that gives a zero drain timeout where `new()` gives 30 s.
4. **Specific boundary holes.** A TLS chain path without a key silently means plaintext
   (`serve.rs:3932`); an empty token variable is a valid token; `FetchSince { after: u64::MAX }`
   overflows (`changelog.rs:414`); the server kept the weak file-name rule the client replaced; the
   snapshot handle is not from a secure random source (`snapshot.rs:206`).
5. **Three adapters are 80-88% copies of each other and the binaries report startup errors with
   `panic!`** (58 hits). Each durability fix is made three times. No Windows CI at all.

### rusty_bullet

1. **Per-car state is 13 parallel vectors and one is public**
   (`rb_physics_bullet/src/world.rs:339-441`). Out-of-range indexes are ignored by some setters and
   panic in others (`:842-856` against `:924`, `:953`), and `rb_env::Env::step` passes caller input
   straight to a panicking one.
2. **File input is trusted where network input is checked.** Capture timestamps become the physics
   `dt` unchecked (`world.rs:1876`); scenario JSON defaults every field and accepts unknown keys
   (`rb_scenario/src/lib.rs:14-69`); a tolerance check exists for one CLI mode and is missing in two.
3. **`rb_tape_bot` sits outside the project's own lint regime.** No `[lints]` table, 13 `expect`s,
   two `u32` frame subtractions that panic in debug if the counter restarts (`main.rs:82`,
   `rb_tape_hive.rs:88`), bad CLI values silently defaulted, and the tape gate written twice.
4. **Errors are stringly and overloaded.** `Io(String)` drops path and source; variants are reused
   for unrelated failures; four domain outcomes are told apart by `io::ErrorKind`
   (`rb_rlbot_core/src/server.rs:544-556`); a zero-frame score equals a perfect one.
5. **`rb-verify`'s `main` is ~343 lines of hand-rolled dispatch with no unit tests**
   (`rb_verify_cli/src/main.rs:598-940`), with an unknown flag opened as a replay file and five
   undocumented environment variables, one read inside a library function. `PhysicsWorld::step` is
   ~380 lines.

## Cross-cutting read

- **Panic discipline is good everywhere it is linted, and by habit where it is not.** board, mux and
  the 12 `rb_*` crates have zero `unwrap`/`panic!` in non-test code. The exceptions are exactly the
  two places with no lint table and no history of review pressure: mmdb's library and binaries, and
  `rb_tape_bot`.
- **The recurring design drift is the same in all four:** lifecycle state held in booleans, parallel
  collections or locals; errors flattened to strings; secrets redacted on the first type and missed
  on the next.
- **Most bounds, timeouts and strict decodes arrived through review or an incident, not first.**
  Every crate's history says so. The section E tables are the checklist those reviews produced.
- **The workspace explains little of the recurrence directly.** What it does explain: lints are per
  crate by design, there is no `clippy.toml`, the shared error, configuration and atomic-file crates
  go unused by these four, and nextest's retries keep sleeps in tests alive.

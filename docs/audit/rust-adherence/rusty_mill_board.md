# Rust adherence audit: `rusty_mill_board`

| | |
|---|---|
| Crate | `rusty_mill_board` (lib + bin `rusty_mill_board`) and its sub-package `rusty_mill_runner` (lib + bin `mill-runner`) |
| Paths | `crates/apps/rusty_mill_board`, `crates/apps/rusty_mill_board/runner` |
| Commit audited | `f80ee7fc5572de65928be3f7df019ae1f056c7e3` |
| Size | 62 `.rs` files, 31,489 lines. Non-test code: 13,142 non-blank lines, 616 functions |
| Tests | 218 `#[test]` in `src/` test modules, 242 in `tests/` and `runner/tests/` (4 `#[ignore]`d) |

**What "library code" means here.** Everything under `src/` and `runner/src/`, both binaries'
`main.rs` included, with each file cut at its first column-0 `#[cfg(test)]` (21 files have one, always
the trailing `mod tests`). Excluded: those test modules, `tests/`, `runner/tests/`, `tests/ui/*.mjs`,
`tools/*.py`. The crate has no examples or benches.

**How counts were produced.**

- Non-test copies: `awk '/^#\[cfg\(test\)\]/{stop=1} {if(stop)print "";else print}' $f` per file
  (line numbers preserved), then `grep -rn` on the copies. Grep counts are exact for the pattern and
  marked "~" where the pattern is only a proxy.
- Clippy, non-test targets only (no `--all-targets`), in a private target directory, never `--fix`:
  `cargo clippy -p rusty_mill_board -p rusty_mill_runner --message-format=json -- -W clippy::<lint> ...`
  with the JSON de-duplicated by (lint, file, line, column). It built in 22 s, so every lint count in
  section B is a clippy count, not a grep estimate.
- Thresholds were set through `CLIPPY_CONF_DIR` pointing at a scratch `clippy.toml` (the repository has
  none): `excessive-nesting-threshold` at 3, 4 and 5; `too-many-lines-threshold` at 60 and at the
  default 100.
- Rustdoc: `RUSTDOCFLAGS="-D warnings" cargo doc -p rusty_mill_board -p rusty_mill_runner --no-deps`.
- Function length and indentation depth: a script over the non-test copies (first line of `fn` to its
  closing brace). Approximate, used only to rank.
- History: `git log --no-merges -- crates/apps/rusty_mill_board` (195 commits) and
  `git show <sha> -- <path>` for the fix commits cited.

Not read: any other crate's source, any other audit report, ADRs and other documents under the crate's
`docs/`, CI workflow files (CI facts come from `_workspace.md` only).

---

## A. Adherence and drift per preference

### 1. Result and `?` over panics

**Adheres.** Non-test code has 0 `.unwrap()`, 0 `panic!`, 0 `unreachable!`, 0 `todo!`, 0
`unimplemented!`, 0 `assert!`. Both `main`s return `ExitCode` from a `run() -> Result<(), String>`
(`src/main.rs:601-613`, `runner/src/main.rs:111-129`). Lock poisoning is never unwrapped: 19 sites map
it to an error, 34 recover the guard with `PoisonError::into_inner`. Slices that could be out of range
are mostly guarded first (`runner/src/main.rs:113-114` checks `first()` before `args[1..]`).

**Drifts.**

- 1 `.expect()` in non-test code, `src/tick.rs:984`, and it runs while the event-log mutex is held
  (`src/tick.rs:980`). If the write-count bookkeeping is ever wrong the panic poisons that mutex, and
  every later store call answers `Storage("lock poisoned")` (`src/tick.rs:622-624`) until restart.
- `runner/src/config.rs:58` indexes `self.from[0]` in a `pub fn` on a struct whose `from` field is
  `pub` (`runner/src/config.rs:21`). Only `parse` (`runner/src/config.rs:82-84`) checks it is non-empty.
- 31 `clippy::indexing_slicing` and 11 `clippy::string_slice` hits. About 20 are `serde_json::Value`
  indexing, which returns `Null` and cannot panic. The real slices are in text scanning:
  `src/sync.rs:179-188`, `src/backfill.rs:149-208`, `src/github.rs:234`. I found none that can go out
  of range or split a character (offsets come from `find` on the same string, and
  `src/backfill.rs:190` uses `to_ascii_lowercase`, which keeps byte offsets), but nothing enforces it.
- Tests use `.expect()` everywhere (~582 in `src/` test modules, ~1,107 in `tests/`); that is the
  intended escape from `unwrap_used`, which CI applies to tests too.

### 2. Ownership and the type system

**Adheres.** `Column` is a validated newtype with `TryFrom<String>` (`src/domain.rs:32`, `:77`).
`Capability`, `MoveRules`, `ProjectStatus`, `CloseReason`, `Shelf`, `View`, `Kind` are enums, not
strings or flags. Card transitions take `&self` and return a new `Card` (`move_to`
`src/domain.rs:1101`, `close` `:1183`, `add_evidence` `:1236`, `comment` `:1507`), so a refused
change leaves nothing half-applied. `BoardError` → HTTP status is one exhaustive `match` with no
wildcard arm (`src/web.rs:322-337`). Credentials have no `Debug` impl (`src/identities.rs:44-45`).

**Drifts.**

- `Card` (`src/domain.rs:555-651`) has 29 `pub` fields and a derived `Deserialize`. How a card ended
  is spread over four fields: `column == DONE`, `done_ms`, `closed`, `evidence`. "Shipped", "prototype"
  and "dropped" are recomputed from them (`src/domain.rs:1222-1230`). Nothing stops a card that is
  both shipped and dropped, done outside Done, or archived while open; the storage adapter has to
  filter one of these by hand (`src/tick.rs:519-527`).
- A summary read returns full `Card`s with body, prompt, links and comments left empty, guarded only
  by a doc comment: "never write one of these cards back" (`src/store.rs:28-37`, `:50-58`).
  `CardSummary` exists (`src/domain.rs:670`) but the store port does not return it.
- Stringly values where a closed set exists: the HTTP method is `&str` and anything unrecognised is
  sent as GET (`src/http.rs:54-59`, `src/tick.rs:104`; 22 literal method strings); the remote task
  status is a `String` compared with `"open"`, `"done"`, `"wontdo"` (`src/tick.rs:463`, `:490`,
  `:568`, `:1420`); settings operations are a second dispatch on string names
  (`src/settings.rs:407-428`) beside the typed op table.
- `priority: u8` "one of `PRIORITIES`" (`src/domain.rs:284`, `:583`) is checked in one function
  (`src/domain.rs:1376`), not by the type.
- Validated-then-public: `Stage` (`runner/src/config.rs:17-45`) and `Roles(pub BTreeMap<..>)`
  (`src/board.rs:38`) can be built without their checks. 312 `pub` fields in non-test code.
- `ApiError.status == 0` means "transport failure" (`runner/src/ports.rs:14-19`,
  `runner/src/runner.rs:188`), a sentinel inside a status code.
- Errors are flattened to text. 83 signatures return `Result<_, String>`; `BoardError::Storage(String)`
  and `Refused(String)` carry no source; 0 `Error::source` impls; `CallError` has no `Display`
  (`src/api.rs:36-42`). One module returns `String` from `apply` and `BoardError` from `undo`
  (`src/retag.rs:237-242`, `:272`). A JSON serialisation failure is reported as `Storage`
  (`src/api.rs:707-709`).
- `BoardError::Invalid(&'static str)` forces limits to be retyped into messages: "at most 8"
  (`src/domain.rs:1260`) beside `MAX_EVIDENCE = 8` (`:396`), "1 to 500" (`src/page.rs:88`).
- The paired `start_write` / `finish_write` (`src/tick.rs:1006-1011`, `:975-1002`) is kept by
  convention, while the same crate uses a drop guard for the same job elsewhere
  (`src/drain.rs:22`, `:67-75`).

### 3. Async only where there is real I/O concurrency

**Adheres.** 0 `async fn`, 2 `.await` in non-test code (`src/http.rs:69`, `src/main.rs:296`). The
crate is synchronous: plain threads, `Mutex`/`RwLock`/`Condvar`, `thread::scope` for parallel stages
(`runner/src/runner.rs:209-215`). Async appears only where a dependency demands it, wrapped in a
private current-thread runtime with the constraint stated (`src/http.rs:1-8`) and for signals
(`src/main.rs:272-303`).

**Drifts.** The blocking client sits behind a `Mutex` (`src/tick.rs:136`, `:154`, `:178`), so all storage
requests in the process are serialised; a 10 s timeout on one (`src/tick.rs:154`) stalls every other
request. That is a throughput ceiling, not a misuse of async.

### 4. Explicit, flat control flow, short functions

**Adheres.** `let ... else` and early returns are the norm (`src/domain.rs:1150-1166`,
`src/web.rs:344-398`). 0 hits for `clippy::cognitive_complexity` at the default 25. Median function is
short: 15 of 616 exceed 50 lines.

**Drifts.**

- `ops()` is one 425-line function (`src/api.rs:712`) holding 35 operations as inline closures, each
  with its own validation and scope filter. `run()` is 198 lines (`src/main.rs:402`). Next:
  `observe` 80 (`src/tick.rs:1017`), `card_of` 76 (`src/tick.rs:500`), `inbox_act` 73
  (`src/board.rs:529`), `take_in` 71 (`src/board.rs:1278`).
- Nesting. `clippy::excessive_nesting` counts the `impl` and `fn` blocks as levels: 109 hits at
  threshold 3, 28 at 4, 3 at 5. Worst: the sync thread body, a `match` in an `if` in a block in a
  `while` in a closure (`src/syncer.rs:53-78`); `from_repos` (`src/projects.rs:142-190`); `validate`
  (`runner/src/runner.rs:115-159`); the triple `for` in `tick` (`runner/src/runner.rs:194-206`).
- `match` on a `bool` 4 times (`src/tick.rs:1200`, `:1241`, `:1271`, `src/milestones.rs:160`).
- Immediately-invoked closures stand in for a named helper (`src/tick.rs:1205-1225`, `:1246-1284`).

### 5. Composition, no premature abstraction

**Adheres.** No inheritance-shaped designs. 8 traits, each a port with at least two implementations
(real and in-memory or fake): `Store`, `MilestoneStore`, `TickApi`, `Forge`, `Escalations`, `Merged`,
`BoardApi`, `Work`. `Board<S: Store>` is composed with builder methods (`src/board.rs:187-304`).

**Drifts.**

- `Store` has 16 methods, 9 with default bodies, most of which scan the whole board or the whole event log (`src/store.rs:35-143`),
  and a hand-written forwarding `impl Store for Box<T>` (`src/store.rs:268-317`). A missing forward
  compiles and silently uses the slow or wrong default; that happened (commit `eeeeb6ca1`).
- Two parallel dispatch tables for one web surface: `api::ops()` and `Settings::call`
  (`src/web.rs:292-298`).
- `Board` is generic over `S`, but every production and surface path uses the one alias
  `Board<Box<dyn Store>>` (`src/api.rs:20`); the generic parameter serves tests only.

### 6. Tests and docstrings

**Adheres.** 460 tests. Failure and race cases are tested on purpose: injected store failures,
barrier-paused commits, reversed completion order (`tests/tick_store.rs`, `tests/take.rs`,
`tests/commit_order.rs`, `tests/shutdown.rs`). `missing_docs` is enforced and
`cargo doc -D warnings` is clean (0 warnings). An in-memory fake of the storage server
(`tests/fake_tick/`) carries the adapter's contract tests.

**Drifts.**

- No test names `Syncer` (`src/syncer.rs`): start, replace, stop and the no-overlap lock are
  untested. `src/http.rs` and `runner/src/http_board.rs` are covered only through end-to-end tests.
- 24 public fallible functions lack an `# Errors` section (`clippy::missing_errors_doc`): 7 required `Store` methods (`src/store.rs:25`, `:27`, `:80`, `:83`,
  `:85`, `:87`, `:104`), all 10 of the runner's `BoardApi` (`runner/src/ports.rs:43-62`), 3 of `Forge`
  (`src/sync.rs:67-71`), and 4 more (`src/board.rs:1208`, `:1225`, `src/domain.rs:1444`, `:1466`).
- 13 sleeps in Rust tests, a few of them inside poll loops (for example `tests/shutdown.rs:195-199`, `tests/take.rs:800`,
  `tests/tick_store.rs:1669`) and many more in `tests/ui/*.mjs` (`tests/ui/priority.mjs:41`). The
  workspace retries tests twice, which hides timing flakes.
- `_workspace.md` lists no CI job that runs the 21 browser checks in `tests/ui/`. Several fix
  commits rest on them (`d154ccdcc`, `07aeb27af`, `7a4bd175e`).
- No property or fuzz tests for the hand-written parsers (`src/sync.rs:165-196`,
  `src/backfill.rs:171-209`, `src/page.rs:50-62`).

### 7. Never silently swallow errors

**Adheres.** Background failures are logged and retried (`src/syncer.rs:64-66`,
`runner/src/main.rs:100-103`, `src/main.rs:477-481`). A failed comment is logged, not dropped
(`runner/src/runner.rs:218-222`). Partial writes have their own error variant with the revision
reached (`src/domain.rs:790-799`, `src/tick.rs:1267-1280`).

**Drifts.**

- A board document that fails to parse is dropped without a log in the listing path
  (`src/tick.rs:778`), so the card lists with no dependencies, prompt or evidence; the single-card
  path reports the same data as an error (`src/tick.rs:879-884`). Unparseable comments are skipped
  the same way (`src/tick.rs:792-797`).
- A malformed request body becomes `Null` and is then reported as a type error on `null`, losing the
  parse error (`src/web.rs:291`). Same on the client side (`runner/src/http_board.rs:58`).
- A panicked stage thread is discarded: `h.join().ok()` (`runner/src/runner.rs:214`). No log, no step.
- The sync thread's `JoinHandle` is dropped (`src/syncer.rs:51-79`). If it panics, syncing stops and
  `repo()` (`src/syncer.rs:103-109`) still reports it running.
- `--env NAME` for a variable that is unset or not Unicode is dropped silently
  (`runner/src/main.rs:83`).
- A schema that fails to serialise becomes `null` (`src/api.rs:88`); a document body likewise
  (`src/tick.rs:325`).
- A request that fails with a storage error is answered 500 with the text but not logged server-side
  (`src/web.rs:303-314`). The whole server has 19 log call sites.
- 11 `map_err(|_| ...)` discard the cause (`clippy::map_err_ignore`), for example
  `src/tick.rs:218`, `src/page.rs:57-59`, `src/api.rs:603`.

### 8. Dependencies, secrets, input validation

**Adheres.** 16 dependencies in the board, 8 in the runner (plus 2 dev-dependencies), all first-party except
`serde`, `serde_json`, `schemars`, `uuid`, `clap`. Token comparison is constant-time and scans every entry
(`src/identities.rs:304-312`, `src/web.rs:169`, `:257`). Secret files must be mode 600
(`src/identities.rs:51-67`). Two config types redact their token in `Debug`
(`src/github.rs:27-36`, `:100`). Token env values are hidden in `--help`
(`src/main.rs:132`, `:169`). An unauthenticated board refuses a non-loopback bind
(`src/web.rs:63`). Children get an allowlisted environment (`runner/src/main.rs:81-84`). Evidence
links, prompts, dependency counts, page limits, cursors and task ids passed to a subprocess are
validated (`src/domain.rs:403-466`, `:1360`, `src/page.rs:84-90`, `src/bbp_cli.rs:100-102`).

**Drifts.**

- `GithubFile` derives `Debug` with a `pub token: String` (`src/settings.rs:82-95`); the sibling type
  for the same token redacts it.
- The shared token, GitHub token and runner token can be passed as command-line arguments
  (`src/main.rs:132-133`, `:169-170`, `runner/src/main.rs:30-31`), although the same file refuses that
  for the storage token because "process listings show" it (`src/main.rs:122-127`).
- The first admin's token is printed to stderr when stderr is a terminal (`src/main.rs:251-258`).
  Deliberate and documented; still a secret in scrollback.
- No length bound on title, body, comment text, tag count or tag length at the domain boundary
  (`src/domain.rs:952-962`, `:1507-1518`, `:357-364`); the storage server's limits are the only ones.
- `board_events` takes `limit: Option<usize>` unchecked (`src/api.rs:595`, `:872`), while `board_list`
  is held to 1..=500.
- `bbp` is run with `Command::output()` and no timeout (`src/bbp_cli.rs:32-35`); a hung child holds a
  request thread, and with it the shutdown drain.

### 9. Explicit configuration, no magic globals, modular layout

**Adheres.** No `static mut`, no `lazy_static`, no global singletons; the one `OnceLock` is a struct
field (`src/settings.rs:353`). The clock is injected (`src/board.rs:118`, `:175`). Every setting is a `clap`
flag with an `env` name. Retired settings fail startup instead of being ignored
(`src/main.rs:52-74`). `lib.rs` is a module index with one line per module (`src/lib.rs:1-21`).
Limits are named constants with a reason (`src/tick.rs:60-79`).

**Drifts.**

- The storage adapter reads the wall clock and mints time-ordered ids directly
  (`src/tick.rs:616-619`, `:953`, `:1022`, `src/domain.rs:898`), bypassing the injected clock, so
  those paths cannot be tested with a fixed time.
- Hard-coded timeouts: 10 s storage (`src/tick.rs:154`), 30 s board client
  (`runner/src/http_board.rs:36`).
- Six files carry most of the weight: `api.rs` 2,879 lines, `domain.rs` 2,182, `board.rs` 1,960,
  `tick.rs` 1,490, `sync.rs` 1,217, `runner.rs` 1,056.
- The CLI `struct Cli` mixes five one-shot maintenance modes with the server flags through
  `conflicts_with_all` lists (`src/main.rs:77-193`) instead of subcommands.

---

## B. Lints

Enforced today for both packages (per `_workspace.md` and each `[lints]` table): clippy default set,
`clippy::unwrap_used`, `clippy::todo`, `missing_docs`, all at warn and `-D warnings` in CI;
`unsafe_code = forbid`; `cargo fmt --check`; `cargo deny`; `cargo shear`. Not listed below.

### Findings a tool could enforce, not enforced today

Counts are non-test code, both packages.

| Lint or check | Hits | Examples | Note |
|---|---|---|---|
| `clippy::expect_used` | 1 | `src/tick.rs:984` | The crate bans `unwrap` but not `expect`. Tests would need `allow-expect-in-tests = true` in a `clippy.toml` (~1,700 test uses) |
| `clippy::panic` | 0 | | Free to turn on |
| `clippy::indexing_slicing` | 31 | `runner/src/config.rs:58`, `src/external.rs:159`, `src/metrics.rs:235` | ~20 are `serde_json::Value` indexing |
| `clippy::string_slice` | 11 | `src/sync.rs:179`, `src/backfill.rs:149`, `src/github.rs:234` | |
| `clippy::too_many_lines` (default 100) | 2 | `src/api.rs:712`, `src/main.rs:402` | 4 at threshold 60: adds `src/tick.rs:500`, `:1017` |
| `clippy::excessive_nesting` | 109 / 28 / 3 at threshold 3 / 4 / 5 | `src/syncer.rs:57`, `src/projects.rs:170`, `runner/src/runner.rs:202` | Needs `excessive-nesting-threshold` in a `clippy.toml`; none exists |
| `clippy::match_bool` | 4 | `src/tick.rs:1200`, `:1241`, `src/milestones.rs:160` | |
| `clippy::missing_errors_doc` | 24 | `src/store.rs:25`, `src/board.rs:1208`, `src/domain.rs:1444` | |
| `clippy::must_use_candidate` | 61 | `src/api.rs:712`, `src/domain.rs:72`, `src/board.rs:162` | 23 `#[must_use]` already written by hand |
| `clippy::map_err_ignore` | 11 | `src/tick.rs:218`, `src/page.rs:57`, `src/api.rs:603` | |
| `clippy::let_underscore_must_use` | 3 | `src/settings.rs:388`, `src/main.rs:602`, `runner/src/main.rs:118` | All three are deliberate "first one wins" |
| `clippy::needless_pass_by_value` | 12 | `src/board.rs:1211`, `src/api.rs:641`, `src/mcp.rs:32` | |
| `clippy::as_conversions` (`cast_possible_wrap` 1) | 2 | `src/bbp_cli.rs:162`, `src/metrics.rs:226` | Everything else already uses `try_from` |
| `clippy::arithmetic_side_effects` | 61 | `src/bbp_cli.rs:162`, `src/board.rs:1788`, `src/backfill.rs:290` | Noisy; revision and offset arithmetic |
| `clippy::wildcard_enum_match_arm` | 3 | `src/board.rs:589`, `:1912`, `src/projects.rs:65` | `:1912` would silently absorb a new error variant |
| `clippy::exhaustive_enums` | 26 | `src/domain.rs:100`, `:200`, `src/api.rs:37` | 0 `#[non_exhaustive]`. Low value while `publish = false` |
| `clippy::significant_drop_tightening` (nursery) | 15 | `src/tick.rs:178`, `src/board.rs:381`, `src/settings.rs:532` | Several are intentional holds; useful as a review list |
| `clippy::format_push_string` | 4 | `src/board.rs:686`, `src/tick.rs:695`, `runner/src/runner.rs:368` | |
| `clippy::ref_option` | 2 | `src/main.rs:241`, `src/settings.rs:592` | |
| `clippy::unused_self` | 2 | `src/settings.rs:542`, `:592` | |
| `clippy::unnecessary_wraps` | 1 | `src/settings.rs:444` | |
| `clippy::redundant_clone` (nursery) | 2 | `src/main.rs:551`, `:572` | |
| `clippy::clone_on_ref_ptr` | 8 | `src/main.rs:505`, `:512`, `:563` | The file mixes `Arc::clone(&x)` and `x.clone()` |
| `clippy::doc_markdown` | 66 | `src/lib.rs:19`, `src/board.rs:932` | Mostly product names without backticks |
| `clippy::partial_pub_fields` | 2 | `src/api.rs:61`, `src/retag.rs:131` | |
| `clippy::allow_attributes_without_reason` | 4 (1 in `src/`, 3 in `tests/`) | `src/inbox.rs:547`, `tests/inbox_ops.rs:4` | Counted by grep; all four are on test code. See the table below |
| Rustdoc `-D warnings` | 0 | | Clean today, not enforced for this crate |
| `[lints] workspace = true` | n/a | `Cargo.toml:18-24`, `runner/Cargo.toml:18-24` | The two packages repeat the same table instead of inheriting one |

Zero hits, so free to enable as guards: `clippy::panic`, `clippy::unwrap_in_result`,
`clippy::panic_in_result_fn`, `clippy::missing_panics_doc`, `clippy::cast_possible_truncation`,
`clippy::cast_sign_loss`, `clippy::fn_params_excessive_bools`, `clippy::too_many_arguments`,
`clippy::type_complexity`, `clippy::cognitive_complexity`, `clippy::print_stdout` and
`clippy::print_stderr` in the library (all 16 prints are in the two `main.rs`), `unreachable_pub`,
`rust_2018_idioms`.

Not enforceable by an existing lint, so listed in C instead: a secret field under derived `Debug`;
a trait forwarder that omits defaulted methods; a discarded `JoinHandle`; a subprocess with no timeout.

### Every `allow` in the crate

| Location | Attribute | Switches off an enforced lint? | Reason given |
|---|---|---|---|
| `src/inbox.rs:547` | `#[allow(clippy::unwrap_used)]` on `mod tests` | Yes, the crate's `unwrap_used = warn` | No reason given. Every other test module in `src/` complies with the lint |
| `tests/inbox_cli.rs:5` | `#![allow(clippy::unwrap_used, clippy::expect_used)]` | `unwrap_used` yes; `expect_used` is not enforced, so that half does nothing | No reason given |
| `tests/inbox_real_bbp.rs:10` | same | same | No reason given |
| `tests/inbox_ops.rs:4` | same | same | No reason given |
| `tests/fake_tick/mod.rs:5` | `#![allow(dead_code)]` | Yes, rustc's default `dead_code` under `-D warnings` | "Each test crate that includes this module uses part of it." (`tests/fake_tick/mod.rs:4`) |

All four `unwrap_used` allows are in the decision-inbox code and its tests (13 + 65 + 25 `.unwrap()`
calls in the three test files). No `#[expect(...)]` is used anywhere.

---

## C. General Rust rules

### Rules

1. When you implement a trait for a wrapper or smart pointer by forwarding, forward every method,
   including those with default bodies, and add a test that calls each one through the wrapper.
   A defaulted method that is not forwarded compiles and runs the default instead of the real one.
2. When a "begin" call must always be matched by an "end" call, return a guard whose `Drop` does the
   end. Do not rely on every caller pairing them, and do not assert the pairing with a panic.
3. Do not panic while holding a mutex whose poisoning you report as a permanent error. Either the
   code under the lock cannot panic, or poisoning is recovered.
4. Choose one poisoning policy per lock and write down why. If the guarded data is always consistent,
   recover the guard; if not, fail. Do not mix the two by habit.
5. When a reduced read fills only part of a record, give it its own type. A full-record type with
   empty fields and a comment saying "never write this back" will be written back one day.
6. When a struct's fields are checked by a constructor or parser, keep the fields private or use a
   type that carries the check. A method that relies on the check must not be reachable on an
   unchecked value.
7. Model a set of mutually exclusive outcomes as one enum, not as several optional fields and a
   status field that must agree.
8. Deserialize a closed set of strings from another system into an enum with an explicit
   unknown case. Do not compare string literals at each use.
9. Do not use a fallback arm that turns an unknown input into a valid default action. An unknown
   verb, mode or kind is an error.
10. Give a type that holds a secret a hand-written or redacting `Debug`, and do it for every type that
    carries the secret, not only the first one.
11. Keep the `JoinHandle` of a long-lived worker thread, or catch its panic and record it. State
    derived from "a worker was started" is not evidence that it is still running.
12. When joining scoped threads, handle the `Err` of `join`. Dropping it turns a panic into a
    missing result.
13. When a shared map keeps "the latest version seen", merge with `max` rather than overwrite.
    Callers finish in a different order than their writes were applied.
14. Do not hold a lock across network or disk I/O. Take a snapshot and an epoch under the lock, do the
    I/O unlocked, then re-take the lock and retry if the epoch moved.
15. When check and commit must agree on shared configuration, hold the read lock from the check
    through the commit, and derive, validate, persist and publish a configuration change under one
    write lock.
16. Map errors to transport status codes in one exhaustive `match` with no wildcard arm, and make
    that one function reachable by every client and test that needs it. A second copy drifts each time
    a variant is added.
17. Do not turn a parse failure into a null or default value and carry on. Return the parse error
    where it happened; the later error is about the wrong thing.
18. Substitute placeholders in one pass over the template. Chained `replace` calls re-scan text that
    an earlier replacement inserted.
19. Code under `#[cfg(...)]` needs its imports under the same `cfg`, and must be compiled and linted
    on every target it claims. An import used only by one platform's branch is an unused-import
    error on the others.
20. Inject time and id generation through the same seam everywhere. One direct read of the system
    clock in an adapter makes that path untestable with a fixed clock.
21. Keep string-offset arithmetic on the string the offsets came from. If you search a transformed
    copy and slice the original, use a transformation that preserves byte offsets, and say so.

### Evidence

| Rule | File:line or commit | What was seen |
|---|---|---|
| 1 | `eeeeb6ca1`; `src/store.rs:268-317` | The boxed store did not forward one defaulted method, so production used the in-memory default instead of the durable history. Fixed by adding the forward. All 16 are forwarded now, by hand |
| 2 | `src/tick.rs:975-1002`, `:1006-1011`, `:1204`, `:1225` vs `src/drain.rs:22`, `:67-75` | Manual `start_write` / `finish_write` with `.expect("a finished write was marked")`; the same crate uses a drop guard for in-flight counting |
| 3 | `src/tick.rs:980-984`, `:622-624` | `expect` under the event-log lock; poisoning there is mapped to a storage error on every later call |
| 4 | `src/store.rs:213-215`, `src/tick.rs:622-624` (19 sites fail) vs `src/drain.rs:25`, `src/board.rs:126-129`, `src/syncer.rs:56` (34 sites recover) | Both policies in one crate; only `src/board.rs:125` states a reason |
| 5 | `src/store.rs:28-37`, `:50-58`; `src/domain.rs:670` | `summaries()` and `in_columns()` return `Vec<Card>` with fields left empty; a summary type exists but is not the return type |
| 6 | `runner/src/config.rs:21`, `:58`, `:82-84`; `src/board.rs:38` | `from[0]` on a public field validated only in `parse`; `Roles` has a public inner map beside a `check_roles` function |
| 7 | `src/domain.rs:626-647`, `:1222-1230`; `src/tick.rs:519-527` | `done_ms`, `closed`, `evidence`, `archived_ms`, `restored_ms` and `column` jointly encode the end state; the adapter filters an impossible combination by hand |
| 8 | `src/tick.rs:463`, `:490`, `:568`, `:1420` | Remote status compared as string literals in four places; any unknown status reads as Done |
| 9 | `src/http.rs:54-59` | `_ => self.client.get(url)`: an unknown method string is sent as GET |
| 10 | `src/github.rs:27-36`, `:100` vs `src/settings.rs:82-95` | Two types redact the token; a third, holding the same token in a `pub` field, derives `Debug` |
| 11 | `src/syncer.rs:51-79`, `:103-109` | Handle dropped at spawn; `repo()` reports from the stored flag, not from the thread |
| 12 | `runner/src/runner.rs:214` | `h.join().ok()` inside `filter_map` |
| 13 | `558a07ff7`; `src/tick.rs:993-1000` | "keep seen versions monotonic": `insert` replaced by `and_modify(max)` after callers were shown to return in reverse order |
| 14 | `61bdc270d`; `src/tick.rs:1018-1045` | "serialize writes with observation": snapshot plus `write_epoch`, fetch unlocked, retry if the epoch moved |
| 15 | `474312301`, `61a4d70ab`, `b6fa5b08e`; `src/board.rs:359-376` | Gate added between a move's check and commit; a project addition derived from a read-locked snapshot was lost |
| 16 | `e68937b61`, `91200c2d8`; `src/web.rs:322-337`; `runner/src/runner.rs:413` | The server's mapping is private; the runner's in-process test API kept a copy that needed a follow-up commit for each new variant |
| 17 | `src/web.rs:291`, `runner/src/http_board.rs:58`, `src/api.rs:88`, `src/tick.rs:325`, `:778` | `unwrap_or(Value::Null)` four times; `.ok()?` drops an unparseable stored document from listings |
| 18 | `runner/src/runner.rs:376-386` | `{prompt}` is replaced first with card text, then `{id}`, `{title}`, `{worktree}` are replaced in the result, so a card body containing `{worktree}` is expanded |
| 19 | `6bf496656`, `b32bd1044`, `f28a4bf04` | Three separate commits to fix an import or helper used only under `cfg(unix)` that failed Windows clippy |
| 20 | `src/board.rs:175` vs `src/tick.rs:616-619`, `:953`, `:1022`, `src/domain.rs:898` | Clock injected into the service, read directly in the adapter and for event ids |
| 21 | `src/backfill.rs:190-197`, `:148-155` | Offsets found in a lowercased copy are used on the original; safe only because the copy is ASCII-lowercased. No comment says so |

---

## D. Repo facts

| Fact | Proof |
|---|---|
| `unwrap` is banned in tests as well as code, because CI runs clippy with `--all-targets -D warnings`. Tests use `.expect("...")`. The decision-inbox tests opt out with a file-level allow instead | `Cargo.toml:22-24`; `tests/inbox_ops.rs:4`; `_workspace.md` |
| `expect_used` and `panic` are not enforced for this crate, though a neighbouring project in the workspace enforces both | `_workspace.md` "Not enforced anywhere" |
| The runner is a separate package with the same hand-copied `[lints]` table; the workspace lint table is opt-in and neither package opts in | `Cargo.toml:18-24`, `runner/Cargo.toml:18-24`, `_workspace.md` |
| Cards live in an external task service. Board state is encoded in reserved tag prefixes on the task (`agent`, `board:`, `claim:`, `number:`, `closed:`, `archived:`, `restored:`) plus one JSON "board document" per card. Users may not set those tags | `src/tick.rs:462-534`; `src/domain.rs:357-364` |
| The board document keeps unknown fields and writes them back, so an older binary does not erase a newer one's data | `src/tick.rs:284-287`, `:314-322`; commit `c8e70d6b6` |
| A card's `rev` is the remote task version minus one. An edit touching both the task and its document is two fenced requests and can end as `PartlySaved` | `src/tick.rs:507`, `:1229-1283`; commits `98b1477dc`, `ad93d084b`, `812962055` |
| The event log exists only in memory. A cursor from before a restart is answered with `resync`. Events for outside edits are synthesised by polling | `src/store.rs:17-22`, `:89-99`; `src/tick.rs:577-599`; commit `f650bc0a2` |
| The remote service reads only the first `after` query parameter, so a window start must ride the page cursor | `src/tick.rs:681-682`; commit `1b568dcd7` |
| Card numbers are reserved as one document per number with a derived id and `If-None-Match: *`, never released | `src/tick.rs:81-87`, `:810-829`; commit `22a35d8f0` |
| The server always runs `Board<Box<dyn Store>>`. A test of an adapter must go through the box to prove anything about production | `src/api.rs:20`; commit `eeeeb6ca1` |
| Lock order in `Board`: projects, then statuses. Moves hold the workflow read lock through commit | `src/board.rs:227-228`; commit `474312301` |
| A credential fixes actor and role. A caller may only append `/label` to the actor name; it changes attribution, never authority | `src/api.rs:119-137`; `src/identities.rs:81-87`; commit `1290de833` |
| Admin-only operations that carry human authority need a transport-bound identity; a caller-named actor and role are refused | `src/api.rs:160-168`; commits `57114de55`, `9fafef5b3` |
| One actor named by two credentials gets no OAuth identity at all | `src/identities.rs:292-301`; commit `8452ee5f5` |
| The decision inbox talks to `bbp` as a subprocess, never by reading its files, and writes its Remediate card before sending an irreversible drop; a memory-only store refuses that action | `src/bbp_cli.rs:1-5`; `src/store.rs:139-143`; commits `1ae8b72c3`, `5876350f6` |
| Retag records are refused on non-Unix because the parent directory cannot be synced there | `src/retag.rs:210-227`; commit `2ad0c3002` |
| Retired environment variables stop startup rather than being ignored, because ignoring them would start an empty in-memory board | `src/main.rs:52-74`; commit `f680de284` |
| The storage token is environment-only; the other three tokens also accept a flag | `src/main.rs:122-127`, `:132-133`, `:169-170`; `runner/src/main.rs:30-31` |
| The web UI is one 114 KB `ui.html` compiled in with `include_str!`, with two fonts via `include_bytes!`. A Rust test checks the page only asks for fonts the binary serves | `src/web.rs:31-44`, `:409-419` |
| Browser checks are Node scripts in `tests/ui/`; `_workspace.md` lists no CI job for them. The only crate-specific CI job runs `inbox_real_bbp` against a real host | `tests/ui/`; `_workspace.md` |
| The runner names a card's worktree and branch by the first 8 hex digits of its id, so two cards sharing them would share a worktree | `runner/src/git_work.rs:79-81`, `:85-91` |
| The runner counts attempts by scanning the card's comments for the text `[stage] attempt` written by its own actor | `runner/src/runner.rs:348-357` |
| The runner re-enters its own binary as `__sandbox` before argument parsing | `runner/src/main.rs:112-117` |
| `rusty_request` is async-only; this crate wraps it in a private current-thread runtime, one connection per request, and must never be called from inside another runtime | `src/http.rs:1-8`, `:31-40` |
| Tests that write and then execute a script are serialised with a process-wide mutex (ETXTBSY under parallel fork) | `tests/inbox_cli.rs:20-26`; commit `969196238` |

---

## E. Non-Rust rules

### Storage and durability

1. Commit a state change and its log entry as one durable record, and assign the log id inside the
   commit lock. An id taken before the lock can land behind a reader's cursor.
2. Allocate identifiers that must be unique through the store, with a create-if-absent write. A
   process-local `max + 1` is not unique across processes or across deletions.
3. When one logical write needs two requests, validate everything the second request can refuse
   before sending the first, and report a split as its own outcome with the point reached.
4. Before an irreversible action in another system, durably record the intent first, keyed so a retry
   finds the same record. Refuse the action if the store cannot survive a restart.
5. A journal used for undo must be durable, directory entry included, before the first change, must
   record begin and done per item, and undo must touch only items recorded done and unchanged since.
6. When a reader and writer of a stored document can be different versions, keep and write back the
   fields you do not know.
7. Do not silently drop a stored record you cannot parse from one read path while another read path
   reports it as an error.

### Concurrency and consistency

8. Run the guard inside the compare-and-swap, against the state the write is conditioned on. A guard
   run on an earlier listing is a race.
9. Make every write to a shared record a compare-and-swap on the version read, including writes
   that change nothing else, and include scope checks in what the version protects.
10. Do not use a writer's timestamp or time-ordered id as a read cursor across writers. Use the
    reader's own clock with an overlap window, and deduplicate.
11. A delta poll must not advance its high-water mark past an item it skipped.
12. Bound every in-memory log that grows with traffic in a long-running service.

### Networking and paging

13. A paging loop must fail if the cursor does not advance or a continuing page is empty.
14. Bound transfer size by paging whole records, never by truncating a response.
15. Do not rely on repeating a query parameter; check how the server reads duplicates.
16. Always give a subprocess or outbound call a timeout when a request thread waits on it.
17. Apply the same limit check to every list-returning operation.

### Security

18. A `Host` header check is not client authentication. Refuse to serve without credentials beyond
    loopback unless the operator says so explicitly.
19. Resolve authority from the credential presented, never from a name the caller or a stored grant
    supplies; if a name maps to more than one credential, fail closed.
20. An operation that carries elevated authority must require a transport-bound identity even when
    the service otherwise accepts self-declared identities.
21. Give child processes an allowlisted environment, not the parent's minus a blocklist.
22. Accept secrets from the environment or an owner-only file, not from command-line arguments, and
    apply that to every secret the program takes.
23. Validate links by structure (scheme, host, exact path shape, no dot segments, canonical numbers),
    not by prefix.
24. Create scratch directories exclusively under a fresh name; never delete and recreate a
    predictable path.

### Service lifecycle

25. On shutdown, close admission first, then stop the listener, then wait for in-flight work under
    one deadline.
26. Fail startup on a configuration key that was retired, when ignoring it would change where data
    is kept.

### API and UX

27. Keep the last good data and mark it stale when a refresh fails. Blank the view only for errors
    that mean the caller may not see it.
28. Reject an argument of the wrong type. Dropping it widens the query without telling the caller.
29. Make a default response small and let the caller ask for the full form.
30. A renderer of untyped pass-through data must skip a malformed item, not abort the list.
31. When calling a fixed public service, do not reuse a base URL configured for a different host.

### Testing

32. Tests that depend on creation order need a clock that advances on every read.
33. For each guard added in a fix, remove it once and confirm a test fails.
34. Replace fixed sleeps with a wait on the condition; a retry policy hides the flake a sleep causes.

### Evidence

| Rule | File:line or commit | What was seen |
|---|---|---|
| 1 | `5ce3df9bc`; `src/domain.rs:894-903` | Card and event were two writes; ids allocated before the commit let a stalled writer land behind a polling reader |
| 2 | `22a35d8f0`; `src/tick.rs:810-829` | Process-local max+1 gave two cards one number across two boards or after a card left and returned |
| 3 | `812962055`, `ad93d084b`; `src/tick.rs:332-338`, `:1267-1280` | A prompt within its own limit exceeded the document limit once escaped, failing after the first request landed |
| 4 | `1ae8b72c3`, `1947e92d2`, `105a4e46c`, `5876350f6`; `src/board.rs:1898-1909` | The record is written before the drop, at an id derived from the exact decision; memory-only store refused |
| 5 | `3670c3549`, `a85648d67`, `ca70497de`, `918128d09`; `src/retag.rs:177-218`, `:272-304` | Four successive fixes to reach create-exclusive, fsync file and directory, begin/done lines, and undo limited to recorded writes |
| 6 | `c8e70d6b6`; `src/tick.rs:284-287` | Unknown fields flattened into a map and written back |
| 7 | `src/tick.rs:778` vs `:879-884` | Listing drops an unreadable document with `.ok()?`; the single-card read returns a storage error |
| 8 | `0038842af`, `e3c12bdd2`, `cae7b8141` | Take candidates came from a stale listing; claim was not conditional on the column the card was read in |
| 9 | `4d8989cff`; `src/tick.rs:1257-1262` | Milestone moved to another project between scope check and write; an empty patch is still sent so the version moves |
| 10 | `a690cb4e0`; `src/tick.rs:586-590`, `:76-79` | A writer with a fast clock hid later comments from readers using the newest id as cursor |
| 11 | `a690cb4e0`; `src/tick.rs:1053-1061`, `:1083` | `high_water` now stops at a task skipped because a write to it was in flight |
| 12 | `src/tick.rs:578-599`, `:602-605` | `log`, `seen` and `comments_seen` only grow for the life of the process; no cap or trim |
| 13 | `1b568dcd7`; `src/tick.rs:705-718` | Page 2 served page 1 again; the loop now errors on a cursor that does not advance and on an empty continuing page |
| 14 | `b6bb09676`; `src/tick.rs:683-727` | "bound tick collection transfers without truncating records" |
| 15 | `1b568dcd7`; `src/tick.rs:681-682` | Two `after=` parameters; the server read the first |
| 16 | `src/bbp_cli.rs:32-35`; contrast `runner/src/git_work.rs:107-116` | `Command::output()` with no limit in the server; the runner's commands have a wall-clock limit |
| 17 | `src/api.rs:595`, `:872` vs `src/page.rs:84-90` | `board_events` limit unchecked; `board_list` limit checked to 1..=500 |
| 18 | `3268cf314`; `src/web.rs:63` | "The Host check is not client authentication" |
| 19 | `8452ee5f5`; `src/identities.rs:292-301` | A grant recorded only the actor; two credentials for one actor could swap authority |
| 20 | `57114de55`, `9fafef5b3`; `src/api.rs:160-168` | A caller-named actor and role were enough to run an admin-wide tool |
| 21 | `1a1e58b24`; `runner/src/main.rs:81-84` | Tests removed six variables and passed the rest, leaking tokens to child processes |
| 22 | `src/main.rs:122-127` vs `:132-133`, `:169-170`, `runner/src/main.rs:30-31` | One token is environment-only for the stated reason; three others accept a flag |
| 23 | `72804826d`, `7271a715e`, `375e305c6`; `src/domain.rs:403-466` | Successive tightening: host required, printable ASCII, no trailing path except a fixed set, no zero-padded numbers, no dot segments |
| 24 | `ea27c30e9`; `tests/inbox_real_bbp.rs` | A predictable temp path was deleted and recreated; now `create_dir` under a fresh name |
| 25 | `8a1a89f7d`, `9e6896216`; `src/drain.rs:84-107` | A request admitted after the drain saw zero could outrun the wait |
| 26 | `f680de284`; `src/main.rs:52-74` | Retired store settings would have started an empty in-memory board |
| 27 | `07aeb27af`, `f1e7c4765` | A failed inbox read blanked the decisions; now kept and flagged, hidden only on 403/422 |
| 28 | `eeeeb6ca1` | A non-integer date bound was dropped, widening the search |
| 29 | `129ee9e24`, `0e4ee9251`; `src/domain.rs:654-664` | Listing defaults to summaries; the full view is asked for |
| 30 | `7a4bd175e` | One odd detail object stopped every later item from rendering |
| 31 | `2279f7007`; `src/github.rs:90` | A lookup of public links used a configurable base that may be an Enterprise host |
| 32 | `7d2f8d387` | Cards made in the same millisecond reordered paging tests |
| 33 | `5f984092e`, `8a1a89f7d` | "Each guard was removed in turn and a test failed" |
| 34 | `tests/shutdown.rs:195-199`, `tests/take.rs:800`, `tests/tick_store.rs:1669`, `tests/ui/priority.mjs:41` | 13 fixed sleeps in Rust tests and more in browser checks; `_workspace.md` records `retries = 2` |

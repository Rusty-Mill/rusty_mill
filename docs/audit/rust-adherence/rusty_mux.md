# Rust adherence audit: rusty_mux

**Crate:** `rusty_mux` (no package of that name: a family of 11 packages `rusty_mux_{client,core,ctl,gui,mcp,proto,script,server,status,transport,tui}`).
**Path:** `crates/apps/rusty_mux/crates/*`
**Commit audited:** `f80ee7fc5572de65928be3f7df019ae1f056c7e3` (HEAD)
**Size:** 167 `.rs` files, 77,665 lines, 399 commits touching the path.

## How the counts were produced

"Library code" below means **non-test code**: everything under `crates/*/src/` except files named
`tests.rs`, anything in a `tests/` directory, and any item under `#[cfg(test)]` or
`#[cfg(all(test, ...))]`. There are no `examples/` or `benches/` directories. Binaries' `main.rs`
count as non-test code.

| What | Command |
|---|---|
| Sizes | `find crates/<pkg> -name '*.rs' \| xargs cat \| wc -l` per package |
| Non-test listing | A throwaway Python script walked `crates/*/src`, skipped `tests.rs` and `tests/`, and dropped every item that follows a `#[cfg(test)]` / `#[cfg(all(test` attribute, finding the item's end by rustfmt indentation. Result: 39,490 non-test lines of 51,363 in `src/` (client 1,485; core 3,141; ctl 9,245; gui 3,027; mcp 259; proto 4,864; script 1,352; server 9,450; status 524; transport 1,474; tui 4,669). Comment-only lines were then removed for pattern counts. |
| Pattern counts ("grep") | `grep -cP '<pattern>'` over that listing, per package. Cross-checked against `grep -rn 'unwrap()\|\.expect(\|panic!\|unreachable!' --include=*.rs .` (2,303 hits in the whole tree, all in test code). |
| Lint counts ("clippy") | `CARGO_TARGET_DIR=<scratch> cargo clippy --locked -p rusty_mux_core -p rusty_mux_proto -p rusty_mux_transport -p rusty_mux_status -p rusty_mux_script -p rusty_mux_client -p rusty_mux_server -p rusty_mux_ctl -p rusty_mux_tui -p rusty_mux_mcp --no-deps --message-format=json -- -W clippy::<lint> ...` (about 100 extra lints at warn), tallied by lint code and de-duplicated by file, line and column. It built and ran in under a minute. Limits: Linux host only, so `cfg(windows)` code was not linted; default features; lib and bin targets only, so test code is excluded by construction; **`rusty_mux_gui` was not linted** (left out of the `-p` list; its numbers are grep-only). |
| Function length and nesting | A second throwaway script over the non-test listing: a function runs from its `fn` line to the closing brace at the same indentation; length is non-blank, non-comment lines; nesting is the indentation of lines that open an `if`/`for`/`while`/`loop`/`match`/arm block. Nesting numbers are approximate. |
| Tests | `grep -rE '^\s*#\[(rusty_tokio::)?test' --include=*.rs <pkg> \| wc -l` |
| History | `git log --no-merges --format='%h %s' -- crates/apps/rusty_mux` (all subjects read), then `git log -1 --format=%b <sha>` for about 70 fix and review commits. Diffs were not read line by line. |

## What was read

| Package | Read in full (non-test code) | Sampled |
|---|---|---|
| transport | all seven modules | none |
| core | `lib`, `entity`, `id`, `identity`, `mux`, `error` | `layout` (parser and `largest_in` read), `event`, `names`, `saved` by grep and clippy |
| proto | `lib`, `error`, `varint`, `field`, `frame`, `args`, `endpoint`, `older`, `checkpoint` | `message.rs` (features, id tables, decode helpers, about a third), `schema.rs`, `theme.rs`, `style.rs` by grep and clippy |
| server | `lib`, `conn`, `state` (doc comments skimmed), `datagram`, `outbox`, `login` | `tailnet` (first half), `pty/*`, `store*`, `spool`, `settings`, `process`, `terminal`, `passed`, `endpoint`, `main` by grep, clippy and targeted reads |
| ctl, tui, gui, client, script, status, mcp | none | grep, clippy (not gui), the length script, and targeted reads of the sites cited |

Test code was counted, not reviewed. `_workspace.md` was the only audit document read.

---

## A. Adherence and drift per preference

### 1. Result and `?` over panics; no unwrap/expect outside tests

**Adheres, completely, and with nothing enforcing it.**

- Non-test code: `unwrap()` 0, `.expect(` 0, `panic!`/`unreachable!`/`todo!`/`unimplemented!` 0, `assert!` family 0 (grep; clippy `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unwrap_in_result`, `missing_panics_doc` all 0 on ten packages).
- Test code holds every one of the 2,303 grep hits.
- One `debug_assert!`: `crates/apps/rusty_mux/crates/server/src/pty/windows.rs:370`.
- Mutex poisoning is recovered, not unwrapped: 11 `lock().unwrap_or_else(PoisonError::into_inner)` sites in gui and tui (grep).

**Residual panic paths the zero count hides:**

- Slice and array indexing: 76 (clippy `indexing_slicing`; tui 23, ctl 16, core 12, transport 10, server 9, proto 5, client 1). Each one read was guarded, but some guards are a call away: `crates/apps/rusty_mux/crates/transport/src/seal.rs:133` indexes the replay bitmap safely only because `open` called `fresh` first (`seal.rs:87`).
- Unchecked integer arithmetic: 249 (clippy `arithmetic_side_effects`; tui 73, transport 71, server 45). `crates/apps/rusty_mux/crates/transport/src/stream.rs:82` (`sent - una - acked.len() - lost.len()`) rests on invariants kept elsewhere in the file.
- `RefCell` borrows: 69 `.borrow()`/`.borrow_mut()` sites (grep), all but one `try_borrow_mut` able to panic on a double borrow. The server's whole state is one `Rc<RefCell<State>>` (`crates/apps/rusty_mux/crates/server/src/state.rs:43`).
- `Duration` subtraction: `crates/apps/rusty_mux/crates/server/src/conn.rs:113`, guarded 32 lines earlier at `conn.rs:81`.

### 2. Ownership and types; illegal states unrepresentable; immutability

**Adheres in the pure crates.**

- Typed ids by macro: `crates/apps/rusty_mux/crates/core/src/id.rs:8`. A block id cannot be passed as a window id.
- Validated constructors with private fields: `Size::new` (`crates/apps/rusty_mux/crates/core/src/entity.rs:18`), `Principal::new` (`crates/apps/rusty_mux/crates/core/src/identity.rs:28`).
- Entities are `pub(crate)` fields with read-only accessors; only `Mux` mutates them (`crates/apps/rusty_mux/crates/core/src/entity.rs:1`). Every `Mux` operation "succeeds and queues its events, or fails and changes nothing" (`crates/apps/rusty_mux/crates/core/src/mux.rs:30`) and the code read holds to it: checks first, mutation last.
- Enums where two booleans would do: `Kept` (`crates/apps/rusty_mux/crates/server/src/state.rs:454`), `Peer` (`crates/apps/rusty_mux/crates/server/src/lib.rs:259`), `Slot` (`crates/apps/rusty_mux/crates/server/src/datagram.rs:67`), `Origin` (`crates/apps/rusty_mux/crates/proto/src/endpoint.rs:37`).
- 0 `static mut`; 2 `static` atomics, both a constant `false` named `NEVER`.

**Drifts in the server and at the wire.**

- **Error kinds travel as string prefixes and are parsed back.** The typed `Error::InputGated` is rendered as `"input_gated: ..."` (`crates/apps/rusty_mux/crates/core/src/error.rs:165`); the server adds `"owner_unreachable: "` (`crates/apps/rusty_mux/crates/server/src/state.rs:2035`), `"bad_launch(env): "` (`state.rs:2508`), `"input_unrecorded: "` (`state.rs:1985`), `"exited: "` (`state.rs:121`); the GUI then branches on `reason.starts_with("input_gated")` (`crates/apps/rusty_mux/crates/gui/src/lease.rs:142`), `reason.starts_with("owner_unreachable")` (`lease.rs:171`) and on English prose, `reason.contains(": acquire its lease")` (`lease.rs:166`). The wire already has a `Code` enum, with two variants (`crates/apps/rusty_mux/crates/proto/src/message.rs:350`).
- **A block's lifecycle is an `Option` plus two booleans.** `BlockRt { live: Option<Live>, hold: bool, released: bool }` (`crates/apps/rusty_mux/crates/server/src/state.rs:191`, `:217`, `:220`); the reason a block is read-only is recovered by testing the booleans in order (`state.rs:318-328`).
- **A client's lifecycle is membership in six maps** keyed by the same id: `clients`, `features`, `tokens`, `orphans`, `stale`, `silenced` (`crates/apps/rusty_mux/crates/server/src/state.rs:402-420`). `drop_client` has to clear each by hand (`state.rs:961-970`). Six fix commits landed in exactly this bookkeeping (see C3).
- **One `Message` enum for both directions.** The server ends its request match with `other => (0, Err("not a client request: ..."))` (`crates/apps/rusty_mux/crates/server/src/state.rs:1304`); clients have `other => Err("expected Welcome, got ...")` (`crates/apps/rusty_mux/crates/client/src/attach.rs:521`, `crates/apps/rusty_mux/crates/ctl/src/lib.rs:1021`).
- **Block methods are dispatched on their name string** after the catalog lookup, with a `_ =>` fallback (`crates/apps/rusty_mux/crates/server/src/state.rs:1540-1632`), so a method added to the catalog and not to the match fails at run time.
- Connection identity is `Rc::ptr_eq` on the connection's cancel `Notify` (`crates/apps/rusty_mux/crates/server/src/state.rs:838`, `crates/apps/rusty_mux/crates/server/src/outbox.rs:127`): one value is both "which connection" and "how to cancel it".
- Same-typed positional parameters: `Mux::grant(owner: ClientId, block, client: ClientId, until: u64, now: u64)` (`crates/apps/rusty_mux/crates/core/src/mux.rs:955`); wall-clock milliseconds are bare `u64` (`crates/apps/rusty_mux/crates/core/src/entity.rs:154`).
- `Result<_, String>`: 169 signatures (grep), 24 of them `pub fn` (client 12, proto 4, server 3, ctl 2, script 2, gui 1), next to the typed `Error` enums in core and proto. Example: `crates/apps/rusty_mux/crates/proto/src/checkpoint.rs:13`, `:31`, `:45`.
- Smaller: `Option<Option<BlockId>>` (`crates/apps/rusty_mux/crates/tui/src/follow.rs:27`); 7 booleans in `struct App` (`crates/apps/rusty_mux/crates/tui/src/main.rs:1477`); tuple aliases `Released = (ClientId, BlockId, Rc<Notify>)` (`crates/apps/rusty_mux/crates/server/src/state.rs:47`); `DatagramKey(pub [u8; 32])` with a public field (`crates/apps/rusty_mux/crates/proto/src/message.rs:331`).

### 3. Async only where there is real I/O concurrency

**Adheres.** All 57 `async fn` and all 98 `.await` are in `rusty_mux_server`, which multiplexes PTYs, sockets, UDP and timers on one `LocalRuntime` thread (`crates/apps/rusty_mux/crates/server/src/lib.rs:14`). Core, proto, transport, status and script are sans-I/O by design and say so in their manifests; transport takes the clock and bytes as arguments (`crates/apps/rusty_mux/crates/transport/src/lib.rs:11`). The clients (ctl, tui, gui, client) use blocking sockets and 23 `std::thread` spawns, no runtime. Clippy `unused_async`: 1, a platform-parity stub (`crates/apps/rusty_mux/crates/server/src/pty/linux.rs:178`).

**Drift:** blocking calls on the runtime thread. `Process::drop` on Linux does a blocking `wait()` after SIGKILL (`crates/apps/rusty_mux/crates/server/src/pty/linux.rs:183-197`). History shows the same class fixed twice elsewhere (`eee2d8726`, `2801454e9`).

### 4. Explicit, flat control flow; short functions

**Mostly adheres.** Of 1,724 non-test functions (script count), 1,681 are 50 lines or fewer. Guard clauses and `let ... else` are the house style throughout core and server.

**Drift (approximate, script):**

- Over 50 lines: 43. Over 80: 14. Over 100: 10 (clippy `too_many_lines`, threshold 100: 9 on ten packages).
- Longest: `encode_body` 325 (`crates/apps/rusty_mux/crates/proto/src/message.rs:1077`), `apply` 275 (`crates/apps/rusty_mux/crates/server/src/state.rs:1028`), `decode_body` 255 (`crates/apps/rusty_mux/crates/proto/src/message.rs:1403`), `put_event` 204 (`crates/apps/rusty_mux/crates/proto/src/message.rs:1958`), `describe` 203 (`crates/apps/rusty_mux/crates/ctl/src/render.rs:266`), `attach` 186 (`crates/apps/rusty_mux/crates/tui/src/main.rs:904`), `apply` 133 (`crates/apps/rusty_mux/crates/gui/src/session.rs:432`), `block_call` 123 (`crates/apps/rusty_mux/crates/server/src/state.rs:1504`).
- Nesting over three levels: ~37 functions; over four: ~7. Deepest: `has_host` (`crates/apps/rusty_mux/crates/ctl/src/ssh.rs:211`, six), `read_one` (`crates/apps/rusty_mux/crates/gui/src/source.rs:1237`), `open` (`crates/apps/rusty_mux/crates/ctl/src/own.rs:466`), `next` (`crates/apps/rusty_mux/crates/server/src/outbox.rs:175`).
- `match` on a `bool`: 51 (clippy `match_bool`; ctl 22, tui 13). Example: `crates/apps/rusty_mux/crates/server/src/conn.rs:268`.
- Unrelated shadowing: 42 (clippy `shadow_unrelated`).

### 5. Composition over inheritance; no abstraction before two call sites

**Adheres.** Two traits in 39k lines (`crates/apps/rusty_mux/crates/server/src/passed.rs:37`, `crates/apps/rusty_mux/crates/tui/src/main.rs:190`), 4 `Box<dyn>`. Variation is passed as closures: `serve_for(..., allowed: impl Fn(&UnixStream) -> io::Result<Peer>, ...)` (`crates/apps/rusty_mux/crates/server/src/lib.rs:298`), `default_socket(env: impl Fn(&str) -> Option<OsString>)` (`crates/apps/rusty_mux/crates/proto/src/endpoint.rs:30`). `Features` is a hand-written bit set (`crates/apps/rusty_mux/crates/proto/src/message.rs:17`), which the workspace's first-party dependency rule explains.

**Drift, minor:** seams that exist only for tests and have one production caller: `watch_with(command, spawn, pidfd_of)` (`crates/apps/rusty_mux/crates/server/src/login.rs:561`), and `#[cfg(test)]` fields and accessors inside production types (`crates/apps/rusty_mux/crates/tui/src/main.rs:1558`, `crates/apps/rusty_mux/crates/server/src/state.rs:636-664`).

### 6. Tests for non-trivial logic including failure and boundary cases; docs on the public surface

**Tests adhere.** 1,146 test functions (server 337, ctl 267, tui 160, core 109, proto 73, gui 56, transport 43, client 32, script 26, status 23, mcp 20). 38,175 of 77,665 lines are test code. Fix commits routinely name the regression test and say it fails on the old code (`989791918`, `cf0998290`, `86e55807a`, `a7c9fe8c4`). Failure and boundary cases are the norm: 2^20 repeated fields, a FIFO swapped for a config file, a full accept backlog, lost datagrams.

**Test drift:**

- 87 `sleep` calls in test code (server 38, tui 32, ctl 14, mcp 2, gui 1). At least five commits repaired timing-dependent tests (see E, Testing).
- No property-based or fuzz dependency in any manifest, for a hand-written binary decoder and a hand-written reliable transport.
- 2 `#[ignore]`, both with a reason (`crates/apps/rusty_mux/crates/server/tests/login.rs:737`, `:783`: need root).

**Docs: mixed.** Every `lib.rs` and `main.rs` opens with a `//!` crate doc, and module docs are unusually good. But:

- rustc `missing_docs`: 520 on ten packages (proto 223, core 163, ctl 61, client 31, tui 19, script 11, status 7, server 5). Most are enum variants, public constants and one-line accessors (`crates/apps/rusty_mux/crates/core/src/entity.rs:25`).
- clippy `missing_errors_doc`: 118 (core 50, client 25, server 16, proto 11).
- By script, 556 of 745 `pub` items have a `///` line directly above.

### 7. Never silently swallow errors

**Mostly adheres.** Discards are deliberate and almost always carry a comment saying why: `crates/apps/rusty_mux/crates/client/src/ssh/forward.rs:103`, `crates/apps/rusty_mux/crates/ctl/src/lib.rs:718`, `crates/apps/rusty_mux/crates/server/src/settings.rs:57`, `crates/apps/rusty_mux/crates/ctl/src/own.rs:328`. Model refusals the server did not expect are logged through one helper (`crates/apps/rusty_mux/crates/server/src/state.rs:2636`). Encoding failures drop the client and log (`state.rs:2421-2428`).

**Counts (non-test):**

- `let _ =`: 87 (grep; clippy `let_underscore_must_use` 63 on ten packages: tui 26, ctl 18, server 11, client 7).
- `.ok()`: 85 (grep). `map_err(|_| ...)`: 39 (grep; clippy `map_err_ignore` 36, proto 22).
- `eprintln!`: 101 (grep; clippy `print_stderr` 75 on ten packages, server 53). It is the only logging; no log crate is used.

**Drift:**

- Error value dropped from the log: `crates/apps/rusty_mux/crates/gui/src/source.rs:1252` and `:1261` print "a bad checkpoint" and discard the reason `Receiver::push` returned.
- A blanket lossy conversion: `impl From<rusty_wire::Error> for Error` maps every wire error to `Truncated` (`crates/apps/rusty_mux/crates/proto/src/error.rs:50-54`).
- Decoders that return `Option`, so the caller cannot say why: `args::decode` (`crates/apps/rusty_mux/crates/proto/src/args.rs:71`), `Chunk::decode_all` (`crates/apps/rusty_mux/crates/transport/src/chunk.rs:103`), `Sealer::open` (`crates/apps/rusty_mux/crates/transport/src/seal.rs:78`; four distinct refusals become one `None`).
- `crates/apps/rusty_mux/crates/server/src/lib.rs:306-310`: the listener's own path silently becomes `None` on any error, and blocks then start without `RUSTY_MUX_SOCKET`.

### 8. Minimal dependencies; no secrets logged; input validated at the boundary

**Dependencies adhere.** core depends on one crate, transport on two, proto on three. No crates.io dependency in ten of eleven manifests. **Drift:** `rusty_mux_server` pulls `uuid`, `serde` and (Linux) `serde_json` beside the first-party `rusty_uuid`, so two UUID types live in one crate (`crates/apps/rusty_mux/crates/server/src/store/record.rs:15`); ctl, gui, tui and mcp take the whole server as a dev-dependency.

**Validation adheres strongly, and history shows it was learned in review.** Frame length checked before buffering (`crates/apps/rusty_mux/crates/proto/src/frame.rs:40`); fixed slots per field id (`crates/apps/rusty_mux/crates/proto/src/field.rs:43`); `MAX_ARGS` checked before the extra argument is read (`crates/apps/rusty_mux/crates/proto/src/args.rs:75`); word count checked against bytes left before allocating (`args.rs:102`); names capped at 256 bytes (`crates/apps/rusty_mux/crates/core/src/mux.rs:15`); peer version restricted to printable ASCII (`crates/apps/rusty_mux/crates/proto/src/message.rs:272`); environment names and descriptor counts checked (`crates/apps/rusty_mux/crates/server/src/state.rs:2505`, `:2518`); terminal size checked before spawning (`crates/apps/rusty_mux/crates/proto/src/checkpoint.rs:13`); config map owner, mode and size checked on the open handle (`crates/apps/rusty_mux/crates/server/src/login.rs:648`).

**Secrets: mostly adheres.** `DatagramKey`'s `Debug` prints `DatagramKey(..)` (`crates/apps/rusty_mux/crates/proto/src/message.rs:333`). Takeover tokens are compared in constant time (`crates/apps/rusty_mux/crates/server/src/state.rs:797`). **Drift:** `Takeover` derives `Debug` with its token (`crates/apps/rusty_mux/crates/proto/src/message.rs:341`), `Message` is printed whole with `{:?}` in three places (`crates/apps/rusty_mux/crates/server/src/conn.rs:51`, `crates/apps/rusty_mux/crates/client/src/attach.rs:521`, `crates/apps/rusty_mux/crates/ctl/src/lib.rs:1021`), and `conn.rs:51` prints a peer-chosen frame of up to 16 MiB to stderr before any handshake.

### 9. Explicit configuration, no magic globals; modular layout

**Layout adheres at the package level.** Eleven packages with one job each; five are pure and say "No I/O" in their manifest. No mutable globals. Pure functions take the environment as an argument (`crates/apps/rusty_mux/crates/proto/src/endpoint.rs:30`, `:48`).

**Drift:**

- `crates/apps/rusty_mux/crates/tui/src/main.rs` is 4,975 lines: the whole client is one inline `mod client` (`crates/apps/rusty_mux/crates/tui/src/main.rs:99`) with its 2,000 lines of tests nested inside (`:2941`). `crates/apps/rusty_mux/crates/server/src/state.rs` is 4,046 lines, 2,700 of them non-test, one `impl State`.
- Environment variables read mid-file rather than at the edge: `RUSTY_MUX_SSH` (`crates/apps/rusty_mux/crates/tui/src/main.rs:644`, `:825`), `RUSTY_MUX_REMOTE_CTL` (`:818`), `RUSTY_MUX_UDP` (`:1111`), `RUSTY_MUX_PREDICT` (`:1210`), `RUSTY_MUX_KEYLOG` (`:1219`); a library function reads one (`crates/apps/rusty_mux/crates/server/src/lib.rs:113`). 28 `env::var` sites in all.
- Protocol tuning is compile-time constants scattered by module (`ORPHAN_GRACE`, `HISTORY`, `BUDGET`, `LEASE_SILENCE`, `OFFER_TTL`, `MAX_ASSOCIATIONS`); only `Limits` (`crates/apps/rusty_mux/crates/server/src/login.rs:97`) and `Udp::idle` are configurable.

---

## B. Lints

None of the eleven packages has a `[lints]` table or opts into `[workspace.lints]`, so each gets only clippy's default set under `-D warnings` and rustfmt. Everything below is **not enforced today**. Counts are clippy on ten packages (not gui), Linux, non-test code, unless marked "grep".

### Already clean: turning these on costs nothing

| Lint | Hits |
|---|---|
| `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::unreachable`, `clippy::todo`, `clippy::unimplemented`, `clippy::dbg_macro`, `clippy::exit`, `clippy::unwrap_in_result`, `clippy::get_unwrap` | 0 each |
| `clippy::undocumented_unsafe_blocks` (the workspace lint this family does not inherit), `clippy::multiple_unsafe_ops_per_block` | 0 each |
| `clippy::cognitive_complexity`, `clippy::fn_params_excessive_bools`, `clippy::mem_forget`, `clippy::large_futures`, `clippy::redundant_clone` | 0 each |
| `unsafe_code` as `forbid` in core, proto, transport, status, script, client, mcp, tui | 0 in those eight (grep also finds none in gui) |

### Findings

| Lint | Hits | Examples |
|---|---|---|
| `missing_docs` (rustc) | 520 | `crates/apps/rusty_mux/crates/core/src/entity.rs:25`; `crates/apps/rusty_mux/crates/proto/src/message.rs:20` |
| `clippy::arithmetic_side_effects` | 249 | `crates/apps/rusty_mux/crates/transport/src/stream.rs:82`; `crates/apps/rusty_mux/crates/transport/src/assoc.rs:265` |
| `clippy::must_use_candidate` | 133 | `crates/apps/rusty_mux/crates/transport/src/assoc.rs:140` |
| `clippy::missing_errors_doc` | 118 | `crates/apps/rusty_mux/crates/core/src/entity.rs:18`; `crates/apps/rusty_mux/crates/core/src/identity.rs:28` |
| `clippy::missing_const_for_fn` | 94 | `crates/apps/rusty_mux/crates/transport/src/assoc.rs:145` |
| `clippy::as_conversions` | 84 | `crates/apps/rusty_mux/crates/transport/src/assoc.rs:441`; `crates/apps/rusty_mux/crates/transport/src/chunk.rs:70` |
| `clippy::indexing_slicing` | 76 | `crates/apps/rusty_mux/crates/transport/src/seal.rs:133`; `crates/apps/rusty_mux/crates/proto/src/frame.rs:49` |
| `clippy::print_stderr` | 75 (grep: 101 with gui) | `crates/apps/rusty_mux/crates/server/src/conn.rs:51` |
| `clippy::let_underscore_must_use` | 63 (grep `let _ =`: 87 with gui) | `crates/apps/rusty_mux/crates/ctl/src/own.rs:330`; `crates/apps/rusty_mux/crates/tui/src/udp.rs:207` |
| `clippy::doc_markdown` | 54 | `crates/apps/rusty_mux/crates/transport/src/lib.rs:1` |
| `clippy::match_bool` | 51 | `crates/apps/rusty_mux/crates/server/src/conn.rs:268`; `crates/apps/rusty_mux/crates/transport/src/assoc.rs:239` |
| `clippy::wildcard_enum_match_arm` | 46 | `crates/apps/rusty_mux/crates/server/src/state.rs:2589` (a new event kind defaults to "send to every client, old ones included"); `crates/apps/rusty_mux/crates/server/src/state.rs:2679`; `crates/apps/rusty_mux/crates/client/src/attach.rs:521` |
| `clippy::shadow_unrelated` | 42 | `crates/apps/rusty_mux/crates/core/src/mux.rs:1161` |
| `clippy::clone_on_ref_ptr` | 42 | `crates/apps/rusty_mux/crates/tui/src/udp.rs:100` (server 36) |
| `clippy::map_err_ignore` | 36 | `crates/apps/rusty_mux/crates/proto/src/field.rs:109`; `crates/apps/rusty_mux/crates/server/src/tailnet.rs:51` |
| `clippy::cast_possible_truncation` | 35 | `crates/apps/rusty_mux/crates/transport/src/assoc.rs:612` |
| `clippy::semicolon_if_nothing_returned` | 17 | `crates/apps/rusty_mux/crates/tui/src/main.rs:40` |
| `clippy::string_slice` | 12 | `crates/apps/rusty_mux/crates/core/src/layout.rs:644`; `crates/apps/rusty_mux/crates/server/src/terminal.rs:376` |
| `clippy::redundant_closure_for_method_calls` | 10 | `crates/apps/rusty_mux/crates/server/src/state.rs:869` |
| `clippy::too_many_lines` (default threshold 100) | 9 | `crates/apps/rusty_mux/crates/proto/src/message.rs:1077`; `crates/apps/rusty_mux/crates/server/src/state.rs:1028` |
| `unsafe_code` (rustc) | 9 (ctl 2, server 7) | `crates/apps/rusty_mux/crates/server/src/login.rs:515`; `crates/apps/rusty_mux/crates/ctl/src/conn.rs:75`. Every block has a `// SAFETY:` comment. |
| `clippy::needless_pass_by_value` | 8 | `crates/apps/rusty_mux/crates/server/src/store.rs:75` |
| `clippy::similar_names` | 7 | `crates/apps/rusty_mux/crates/core/src/mux.rs:216` |
| `clippy::single_match_else` | 6 | `crates/apps/rusty_mux/crates/proto/src/field.rs:53` |
| `clippy::allow_attributes` and `clippy::allow_attributes_without_reason` | 5 each on Linux | see the list below |
| `clippy::match_same_arms` | 4 | `crates/apps/rusty_mux/crates/tui/src/screen.rs:228` |
| `clippy::print_stdout` | 4 | `crates/apps/rusty_mux/crates/tui/src/main.rs:1093` |
| `clippy::float_arithmetic` | 3 | `crates/apps/rusty_mux/crates/core/src/layout.rs:284` |
| `clippy::unused_self` | 3 | `crates/apps/rusty_mux/crates/server/src/pty/linux.rs:176` |
| `clippy::error_impl_error` | 2 | `crates/apps/rusty_mux/crates/core/src/error.rs:8`; `crates/apps/rusty_mux/crates/proto/src/error.rs:6` |
| `clippy::ref_option` | 2 | `crates/apps/rusty_mux/crates/ctl/src/hosts.rs:547` |
| `clippy::struct_excessive_bools` | 1 | `crates/apps/rusty_mux/crates/tui/src/main.rs:1477` |
| `clippy::option_option` | 1 | `crates/apps/rusty_mux/crates/tui/src/follow.rs:27` |
| `clippy::unchecked_time_subtraction` | 1 | `crates/apps/rusty_mux/crates/server/src/conn.rs:113` |
| `clippy::unused_async` | 1 | `crates/apps/rusty_mux/crates/server/src/pty/linux.rs:178` |
| `clippy::partial_pub_fields`, `clippy::manual_let_else`, `clippy::default_trait_access`, `clippy::trivially_copy_pass_by_ref`, `clippy::many_single_char_names`, `clippy::str_to_string` | 1 each | `crates/apps/rusty_mux/crates/proto/src/schema.rs:221`; `crates/apps/rusty_mux/crates/server/src/state.rs:1548` |
| `clippy::debug_assert_with_mut_call` (nursery; not run) | 0 now, 1 in history | fixed by `c54e8143a`: `debug_assert!(child.disown().is_none())` dropped the call in release builds |
| `clippy::disallowed_methods` via a `clippy.toml` (absent) for `std::env::var`/`var_os` outside `main.rs` | 28 sites (grep) | `crates/apps/rusty_mux/crates/tui/src/main.rs:1210`; `crates/apps/rusty_mux/crates/server/src/lib.rs:114` |

### CI checks not run for this family

| Check | State |
|---|---|
| `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings` | Not run for any `rusty_mux_*` package. Docs carry many intra-doc links (`[`Mux::rename`]`), none verified. |
| `.github/scripts/mux_egress_audit.sh` | Exists; no workflow calls it (`_workspace.md`). |
| Fuzzing or property tests of `rusty_mux_proto::decode` and `rusty_mux_transport::Association::datagram` | None. |
| nextest `retries = 2` | Applies workspace-wide; a test that fails once in three passes. With 87 sleeps in this family's tests, a flaky-test report (tests that passed only on retry) is not produced. |

### Every `allow` in the family

No `#[expect(...)]` anywhere. No `allow` switches off a lint that the crate or workspace enforces beyond clippy's defaults; the two clippy ones switch off a default (`-D warnings`) lint.

| Site | Attribute | Reason given |
|---|---|---|
| `crates/apps/rusty_mux/crates/server/src/state.rs:1804` | `#[allow(clippy::too_many_arguments)]` | **No reason given.** Switches off a default clippy lint that CI denies. The same file already has a parameter struct for the next call down (`Start`, `state.rs:235`). |
| `crates/apps/rusty_mux/crates/ctl/src/lib.rs:585` | `#[allow(clippy::too_many_arguments)]` | "Each is one part of the command." Switches off a default clippy lint that CI denies. |
| `crates/apps/rusty_mux/crates/ctl/src/lib.rs:41` | `#![cfg_attr(not(any(unix, windows)), allow(dead_code))]` | Comment above: the connection code compiles but is never reached off Unix and Windows. |
| `crates/apps/rusty_mux/crates/ctl/src/conn.rs:39` | `#[cfg_attr(not(windows), allow(dead_code))]` | No reason given (doc comment says the item is the Windows timeout). |
| `crates/apps/rusty_mux/crates/ctl/src/hosts.rs:34` | `#[cfg_attr(not(unix), allow(dead_code))]` | No reason given. |
| `crates/apps/rusty_mux/crates/ctl/src/hosts.rs:38` | `#[cfg_attr(unix, allow(dead_code))]` | No reason given. |
| `crates/apps/rusty_mux/crates/server/src/process.rs:43` | `#[cfg_attr(not(target_os = "linux"), allow(dead_code))]` | No reason given. |
| `crates/apps/rusty_mux/crates/server/src/process.rs:68` | `#[cfg_attr(not(windows), allow(dead_code))]` | No reason given (doc comment says it is how a Windows console lists processes). |
| `crates/apps/rusty_mux/crates/server/src/process.rs:206` | `#[cfg_attr(not(target_os = "linux"), allow(dead_code))]` | No reason given. |
| `crates/apps/rusty_mux/crates/server/tests/common/mod.rs:4` | `#![allow(dead_code)]` | No reason given (shared test helpers; each test binary uses a subset). |
| `crates/apps/rusty_mux/crates/tui/tests/common/mod.rs:6` | `#![allow(dead_code)]` | No reason given (same). |

The seven `cfg_attr(..., allow(dead_code))` attributes would be better as `#[cfg(...)]` on the item, or as `#[expect(dead_code, reason = "...")]`, so they fail when the code becomes used.

---

## C. General Rust rules

Habits the code or its history shows were not applied unprompted.

1. Carry an error's kind as an enum value from the place it is raised to the place it is handled. Never render it to text and then match on a prefix or a phrase of that text.
2. When one entity moves through a lifecycle, hold its state in a single enum-valued field or map. Do not spread it across several booleans, an `Option` plus flags, or parallel maps keyed by the same id.
3. When a channel carries messages in two directions, give each direction its own enum, so "a message that cannot arrive here" is a type error and not a wildcard arm.
4. After looking a name up in a table, carry the table's enum or index forward and match on that exhaustively. Do not match on the name again.
5. A future does nothing until polled. If a step must happen even when the task is dropped unpolled, do it in a plain function before the future is created.
6. Give each OS resource its own owning type whose `Drop` releases it fully, and hand it to its long-term owner before anything that can fail. A `Drop` that only checks, and a constructor that can drop the only handle on an error path, both leak.
7. A function that takes ownership of something the caller could also still own, such as a raw descriptor, is `unsafe fn` with its preconditions written down. Keep the safe checks in a separate safe function.
8. Give connection, session or generation identity its own type. Do not compare the address of a shared handle that exists for another purpose.
9. When a function takes two or more arguments of the same primitive type with different meanings, give them distinct types or pass a struct with named fields.
10. Do not write a `From` impl that maps every source error to one variant. Convert explicitly at each site where the collapse is intended, and keep the source otherwise.
11. Public library functions return a typed error. Keep `String` errors for the last step before printing.
12. Never make a blocking system call on the thread that runs a single-threaded executor, `Drop` impls included. Move it to a thread or a blocking pool and apply the result back on the executor thread.
13. Keep `main.rs` to argument parsing and wiring. Put the program in library modules, so tests reach it without `#[cfg(test)]` fields and accessors in production types.
14. In a decoder, return an error that says which check failed, even when the caller will only drop the input. An `Option` leaves nothing to log or count.

### Evidence

| Rule | File:line or commit | What was seen |
|---|---|---|
| 1 | `crates/apps/rusty_mux/crates/core/src/error.rs:165` | A typed variant is displayed as `"input_gated: ..."`. |
| 1 | `crates/apps/rusty_mux/crates/gui/src/lease.rs:142`, `:166`, `:171` | The GUI branches on `starts_with("input_gated")`, `contains(": acquire its lease")`, `starts_with("owner_unreachable")`. |
| 1 | `crates/apps/rusty_mux/crates/server/src/state.rs:1482`, `:2035`, `:2508` | `bad_launch(input)`, `owner_unreachable`, `bad_launch(env)` exist only as string prefixes; `Code` has two variants (`crates/apps/rusty_mux/crates/proto/src/message.rs:350`). |
| 2 | `crates/apps/rusty_mux/crates/server/src/state.rs:191`, `:217`, `:220`, `:318-328` | `live: Option<Live>` with `hold` and `released` booleans; the reason is recovered by testing them in order. |
| 2 | `crates/apps/rusty_mux/crates/server/src/state.rs:402-420`, `:961-970` | Six maps keyed by client id; `drop_client` clears each by hand. |
| 2 | `ce8db1c8c`, `8c7e8ec87`, `8898dbd5c`, `55d4bbf92`, `07267cb9a`, `7d27acc40` | Six fixes to lease, stale and orphan bookkeeping across those maps. |
| 3 | `crates/apps/rusty_mux/crates/server/src/state.rs:1304` | `other => (0, Err("not a client request: ..."))`, answered under request number 0. |
| 3 | `crates/apps/rusty_mux/crates/client/src/attach.rs:521`; `crates/apps/rusty_mux/crates/ctl/src/lib.rs:1021` | `other => Err("expected Welcome, got ...")`. |
| 4 | `crates/apps/rusty_mux/crates/server/src/state.rs:1520`, `:1540-1632` | `schema::block_method(method)` finds the entry, then `match found.name { "terminal.format" => ..., _ => unknown() }`. |
| 4 | `ca500e44e` | An arm for a state the dispatch could never reach had to be removed later. |
| 5 | `86e55807a` | `Process::wait` was `async`; a task dropped before its first poll still held the kill-on-drop child and killed a program that had just been released. |
| 6 | `989791918` | `Pty::drop` only called `try_wait`, so a running shell was never reaped; the fix split the master from an owned `Process`. |
| 6 | `78f1ca99d` | The reaper thread was created after the spawn, so failing to create it dropped the only `Child`. |
| 6 | `face1c7a1` | A watchdog kept a pid after the child was reaped elsewhere. |
| 7 | `b2799d02f`; `crates/apps/rusty_mux/crates/server/src/endpoint.rs:129` | Adopting fd 3 was a safe function until review; now `pub unsafe fn inherited` with a safe `activated` beside it. |
| 8 | `crates/apps/rusty_mux/crates/server/src/state.rs:838`; `crates/apps/rusty_mux/crates/server/src/outbox.rs:127` | A connection is "the same" when `Rc::ptr_eq` holds on its cancel `Notify`. |
| 8 | `07267cb9a` | The client needed a generation number added to ignore messages from a connection it had replaced. |
| 9 | `crates/apps/rusty_mux/crates/core/src/mux.rs:955-962`; `crates/apps/rusty_mux/crates/server/src/state.rs:1261` | `grant(owner, block, client, until, now)`: two `ClientId` and two `u64` by position. |
| 9 | `crates/apps/rusty_mux/crates/proto/src/checkpoint.rs:45` | `push(offset: u64, total: u64, bytes)`. |
| 10 | `crates/apps/rusty_mux/crates/proto/src/error.rs:50-54` | `fn from(_: rusty_wire::Error) -> Self { Self::Truncated }`. |
| 11 | `crates/apps/rusty_mux/crates/proto/src/checkpoint.rs:13`, `:31`, `:45`; `crates/apps/rusty_mux/crates/proto/src/endpoint.rs:48` | `Result<_, String>` in the public API of a crate that has a typed `Error`; 24 such `pub fn` in the family. |
| 12 | `eee2d8726` | A config reload on a stalled filesystem froze every client. |
| 12 | `2801454e9` | Account lookups (which may ask LDAP) and the map read ran on the front door's one thread. |
| 12 | `crates/apps/rusty_mux/crates/server/src/pty/linux.rs:183-197` | `Drop` still does a blocking `wait()` on the runtime thread. |
| 13 | `crates/apps/rusty_mux/crates/tui/src/main.rs:99`, `:2941`, `:1558` | 2,840 non-test lines in one inline module in `main.rs`, tests nested inside, a `#[cfg(test)]` field in the app struct. |
| 13 | `crates/apps/rusty_mux/crates/server/src/state.rs:636-664`; `cf012da32` | Test-only accessors and a "test-only hook" in the state type. |
| 14 | `crates/apps/rusty_mux/crates/transport/src/seal.rs:78`; `crates/apps/rusty_mux/crates/transport/src/chunk.rs:103`; `crates/apps/rusty_mux/crates/proto/src/args.rs:71` | Wrong direction, wrong id, replay and bad tag all return `None`; `Association::datagram` returns a bare `bool` (`crates/apps/rusty_mux/crates/transport/src/assoc.rs:161`). |
| 14 | `crates/apps/rusty_mux/crates/gui/src/source.rs:1252` | The caller logs "a bad checkpoint" with no reason. |

---

## D. Repo facts

| Fact | Proof |
|---|---|
| Non-test code has no `unwrap`, `expect` or `panic!` at all, by habit; no lint holds the line. | grep and clippy counts above; no `[lints]` table in any of the 11 manifests. |
| A model refusal the server did not expect is logged as "model out of step on X" and execution continues. It never panics and never closes the client. | `crates/apps/rusty_mux/crates/server/src/state.rs:2636`; also `:877`, `:894`, `:947`, `:972`. |
| In `rusty_mux_core`, a lookup that "cannot" fail after a check is written `if let Some(..)`/`let .. else { return }`, a silent no-op, not an assertion. | `crates/apps/rusty_mux/crates/core/src/mux.rs:500`, `:520`, `:547`, `:1025`. |
| Adding a wire field means bumping `FIELD_COUNT` by hand; a field id above it is parsed, validated and dropped, so the new field reads as missing. | `crates/apps/rusty_mux/crates/proto/src/message.rs:1016-1017`; `cf0998290`, `12a0c23e9`. |
| Adding a message type or event kind means a new `Features` bit, an entry in `NAMES`, an entry in `OFFERED`, and an arm in `receives`. Only the `NAMES` entry is checked by a test; a missing `receives` arm sends the event to clients that never negotiated it. | `crates/apps/rusty_mux/crates/proto/src/message.rs:279`; `crates/apps/rusty_mux/crates/server/src/state.rs:69`, `:2566-2590`; `0e46d90ec`. |
| Retired wire ids are never reused: message type `0x0d` and feature bits 0-3, 16, 17 stay reserved. | `crates/apps/rusty_mux/crates/proto/src/message.rs:1032`, `:22-33`, `:79-85`. |
| The server is one thread. State is `Rc<RefCell<State>>`; a handler must not hold a borrow across an `await`, and PTY writes are returned as a `Pending` value to perform after the borrow ends. | `crates/apps/rusty_mux/crates/server/src/lib.rs:14`; `crates/apps/rusty_mux/crates/server/src/state.rs:163-185`, `:985-1026`. |
| Requests on one connection must finish in order; anything slow hands the connection a completion to await. | `crates/apps/rusty_mux/crates/server/src/state.rs:177`; `d768fbc04`. |
| The server authenticates by peer credentials only (uid on Linux, token SID on Windows), whatever the socket's file mode. A `Principal` is what the server verified, never what a client says. | `crates/apps/rusty_mux/crates/server/src/lib.rs:145-148`; `crates/apps/rusty_mux/crates/core/src/identity.rs:3`; `99b27b51e`. |
| Logging is `eprintln!` with a `rusty-mux-server:` / `rusty-mux:` / `rusty-mux-gui:` prefix. The workspace's `rusty_log` is not a dependency of any package. In login mode the account server's stderr is `/dev/null`, so those lines go nowhere. | 101 sites; `crates/apps/rusty_mux/crates/server/src/login.rs:506-510`. |
| `rusty_mux_server` uses two UUID crates: `rusty_uuid` everywhere and crates.io `uuid` (with `serde`) for the store's records, because the storage engine's traits need them. | `crates/apps/rusty_mux/crates/server/Cargo.toml`; `crates/apps/rusty_mux/crates/server/src/store/record.rs:14-15`. |
| Platform code is gated `cfg(any(target_os = "linux", windows))`; other targets compile to stubs that return `Unsupported`. CI lints Linux and Windows on a PR, which is why seven items carry `cfg_attr(..., allow(dead_code))`. | `crates/apps/rusty_mux/crates/server/src/lib.rs:22-56`, `:457-494`; allow list in B. |
| `SIGNAL_RELEASE` is offered on Linux only. | `crates/apps/rusty_mux/crates/server/src/state.rs:106-111`. |
| Comments cite decision ids (`C77`, `D164`, `pack D182`) and design documents by path. The code is not fully readable without them. | `crates/apps/rusty_mux/crates/core/src/entity.rs:44`; `crates/apps/rusty_mux/crates/server/src/state.rs:243`. |
| Client error text is part of the contract: the GUI and `rusty-mux-ctl` tests match on it. | `crates/apps/rusty_mux/crates/gui/src/lease.rs:142`; `0e46d90ec` ("ctl tests assert on 'It lacks'"). |
| `rusty-mux-ctl` exit codes are a stable public surface, modelled as an enum. | `crates/apps/rusty_mux/crates/ctl/src/lib.rs:85`. |
| Mutex poisoning is always recovered with `unwrap_or_else(PoisonError::into_inner)`. | 11 non-test sites, e.g. `crates/apps/rusty_mux/crates/tui/src/udp.rs:217`. |
| MSRV differs inside the family: core, proto, script, status declare 1.89; the other seven 1.98.1. | The eleven `Cargo.toml` files. |
| Integration tests of ctl, gui, tui and mcp start a real `rusty_mux_server` in-process (dev-dependency); the two root-only login tests are `#[ignore]`. | manifests; `crates/apps/rusty_mux/crates/server/tests/login.rs:737`. |
| Tests wait on a marker in screen text, outside OSC strings; waiting on raw output or on a title has failed on fast runners. | `53eb0727c`, `e2002a9b7`, `500097e87`. |
| `mux_egress_audit.sh` exists and is not wired into CI. | `_workspace.md`. |

---

## E. Non-Rust rules

### Security

1. Check a file's type, owner and mode on the handle you opened, not on the path, and open it so a FIFO or device cannot block you.
2. When a privileged process creates a path, check every existing component before creating and check all of them again after. Refuse symbolic links in the path instead of resolving them.
3. Set a file's or socket's permissions as it is created. Do not create and then change mode.
4. Before starting a less-privileged child, give it fresh standard streams and mark every inherited descriptor close-on-exec.
5. Signal or wait on a process through a handle that cannot be reused (a pidfd, a job object), never through a stored pid.
6. Bind a resumable session token to the identity the transport verified and to the capabilities negotiated. A valid token from someone else, or with different capabilities, starts a new session.
7. Decide identity by a stable identifier, never by a display name or a rendered form.
8. Escape control characters in every string from a peer or a child process before printing it to a terminal or into a line-oriented format.
9. Do not log a peer's message whole. Log its type and size, and keep credentials out of derived debug output.

### Resource bounds and networking

10. Bound every per-peer structure by count as well as by bytes; a byte-bounded input can still expand into millions of small entries.
11. Count resources that are closed but not yet drained against the same limit as open ones.
12. Bound a value before it fans out to every subscriber, not at each subscriber.
13. When a reply would exceed its limit, refuse it whole with an error. Never return a silent prefix.
14. Do not acknowledge what you have not recorded, and do not close a stream until what was written to it is acknowledged.
15. A deadline on a client call must cover connect, read and write, and be checked between reads, not between whole messages.
16. No single bad packet or accept error may end a server. Size the receive buffer for the largest datagram, and sort accept errors into "skip", "pause" and "fatal".
17. A request that grows another party's queue must be refused once that queue is half full, or one client can get another evicted.

### Protocol and API design

18. If a request has no success reply, provide an ordered no-op the client can send after it, so a dependent request is not sent before a refusal could arrive.
19. A client must check the negotiated capabilities before sending anything a peer without them would ignore, and before waiting for an event such a peer never sends.
20. When rotating a token on reconnect, keep accepting the last token the client proved it held until it proves it holds the new one.
21. Validate every part of a compound update before applying any of it. Plan every step of a destructive operation, dry run included, before taking the first.
22. Never reuse a retired message type, field id or capability bit.

### Concurrency

23. Between checking a condition and recording the result, allow no suspension point. Where one is needed, check again afterwards.
24. A result that arrives late must be matched against the connection or generation that asked for it and dropped if that has been replaced.
25. Tear down in dependency order: stop the thing that would act on a resource before releasing the resource.

### Testing

26. Wait for the observable condition with a bounded poll. Never sleep for a fixed time and never rely on a scheduler delay.
27. Look for a test's marker only where the behaviour under test would put it; the same text can appear earlier somewhere else.
28. A regression test must fail on the code before the fix, and the commit should say so.
29. Retry reads interrupted by signals in test clients as production code does.
30. Hold the processes a test starts by handle and end exactly those on failure.

### CLI and UX

31. A reload that fails keeps the last good configuration. Only at first start may a missing file mean "defaults".
32. When editing a file the user owns, change only the bytes you wrote, refuse input that could shadow their entries, and create backups so an existing file is never overwritten.
33. An error message states only what was established: say "nothing is there" only when the lookup said not found.
34. Restore the terminal on every exit path, including the peer going away.

### Evidence

| Rule | File:line or commit | What was seen |
|---|---|---|
| 1 | `73f3b322e`, `0dffa451e`; `crates/apps/rusty_mux/crates/server/src/login.rs:648-656` | Config and account map are opened `O_NONBLOCK` and checked through the handle after a FIFO could be swapped in behind a path check. |
| 2 | `5c96759eb`, `f3000d797`; `crates/apps/rusty_mux/crates/server/src/login.rs:426-441` | The ancestor check ran on the canonical path while the configured one was used; a part missing at first look was never checked. Two review rounds. |
| 3 | `2801454e9` | The front-door socket was chmod-ed after bind on a path another user could swap. |
| 4 | `8a2198e21`, `b2799d02f`, `ac194ca4e` | The account server inherited root's stderr; block programs inherited the systemd listener on fd 3. |
| 5 | `0c8cca398`, `face1c7a1`, `78f1ca99d`; `crates/apps/rusty_mux/crates/server/src/login.rs:617-621` | Tests and a watchdog signalled by pid after the process could have been reaped; the fix requires a pidfd or a job. |
| 6 | `99b27b51e`, `2916649a7`; `crates/apps/rusty_mux/crates/server/src/state.rs:789-808` | A takeover with fewer features still got a checkpoint it could not read; a token was honoured for any principal. |
| 7 | `8a2198e21`, `7bec4d6a1` | `=` routed by account name, so a duplicate name in a second account database reached another uid; a test compared an ACL by its rendered text. |
| 8 | `6f2ebc3c7`, `79dab6227`, `0e46d90ec`; `crates/apps/rusty_mux/crates/proto/src/message.rs:272` | Tabs, newlines and ESC in process names, entity names and a peer's version string reached output raw. |
| 9 | `crates/apps/rusty_mux/crates/server/src/conn.rs:51`; `crates/apps/rusty_mux/crates/proto/src/message.rs:341` | A pre-handshake frame is printed with `{:?}`; `Takeover` derives `Debug` with its token, while `DatagramKey` (`:333`) is redacted. |
| 10 | `cf0998290`, `d8bc37986`, `3893a276a`; `crates/apps/rusty_mux/crates/proto/src/args.rs:75`, `:102` | A legal 16 MiB frame of repeated fields kept millions of entries; out-of-order pieces and skipped stream numbers were unbounded. |
| 10 | `d3d327168`, `75cdef749` | No cap on associations; one uid could exhaust the front door's descriptors. |
| 11 | `df458cadf`; `crates/apps/rusty_mux/crates/transport/src/assoc.rs:228-230` | Closed, unread streams kept their buffers without limit. |
| 12 | `109f0ba04` | One oversized name pushed every subscribed client over its queue budget, the requester included. |
| 13 | `a293b78ff`, `79dab6227`; `crates/apps/rusty_mux/crates/server/src/state.rs:2778-2790` | A trimmed capture came back as a silent prefix. |
| 14 | `fc5b347b6`, `d25b964aa`, `0b77448ff`, `d6f0b73ee` | A close was confirmed but not recorded, so late data reopened the stream; closing dropped a request whose datagram was lost. |
| 15 | `a7c9fe8c4`, `8f1268fc2`, `070bce00b`, `38686da73` | Blocking connect ignored the deadline; Windows writes never timed out; a trickled message outlived a 10 s bound. |
| 16 | `92d793da5`, `1d3802d20`, `75cdef749`; `crates/apps/rusty_mux/crates/server/src/datagram.rs:196-205`; `crates/apps/rusty_mux/crates/server/src/login.rs:245-266` | An oversized datagram ended the UDP server on Windows; `EMFILE` from accept ended the front door for every account. |
| 17 | `773f0ddcb`; `crates/apps/rusty_mux/crates/server/src/state.rs:1980-1987`, `:2029-2037` | A flood of lease requests could push the owner over its budget and orphan its blocks. |
| 18 | `0798e4ea7`, `d768fbc04` | `input` then `close-block` closed the block although the input was refused; a pipelined `sync` overtook a reload. |
| 19 | `6b2e88909`, `a971dd8fa`, `61b43cbbc` | A chain ran half its requests before finding the server lacked `SYNC`; `watch` waited for ever for an event an older server never sends. |
| 20 | `55d4bbf92`; `crates/apps/rusty_mux/crates/server/src/state.rs:720-726`, `:992-996` | A reconnect that died mid-handshake lost the client: the token had rotated in a Welcome that never arrived. |
| 21 | `caa33edd8`, `905443929`, `c93fecc7d` | A report with one bad field fell back to the root id and its clear wiped every record; `remove` deleted a file before reading the backup it could not parse. |
| 22 | `crates/apps/rusty_mux/crates/proto/src/message.rs:1032`, `:22-33` | Reserved type `0x0d` and reserved feature bits. |
| 23 | `2801454e9`; `crates/apps/rusty_mux/crates/server/src/login.rs:293-307`, `:690` | A connection was admitted against a map replaced during its lookups. |
| 24 | `07267cb9a`, `ce8db1c8c`; `crates/apps/rusty_mux/crates/server/src/state.rs:566` | A lease granted after focus moved was kept; a lease came back on a connection that no longer served the client. |
| 25 | `face1c7a1`, `8c9e13b5b`; `crates/apps/rusty_mux/crates/client/src/ssh/forward.rs:185-189` | A watchdog outlived the child it guarded; a relay worker outlived its relay. |
| 26 | `500097e87`, `cf012da32`, `316429804`, `807fad095` | Four tests relied on a 50 ms wake delay; one failed 3 of 3 under load; another 1 in 4. 87 sleeps remain in tests. |
| 27 | `53eb0727c`, `e2002a9b7` | A test matched its marker in the window title before the command printed it; another looked only at live output when the text was already in the checkpoint. |
| 28 | `989791918`, `a7c9fe8c4`, `86e55807a` | Each body states the new test fails or hangs on the old code. |
| 29 | `5f4492e42` | A test client failed with `read: Interrupted` in CI. |
| 30 | `0c8cca398`, `52e3b910c` | Cleanup by pid and image name could end another process. |
| 31 | `73f3b322e` | An editor's delete-then-write undid the running settings. |
| 32 | `899ac7401` | `ssh setup --write` could put `Host *` ahead of the user's config and overwrite a backup. |
| 33 | `b2799d02f`, `ac194ca4e` | ctl said "no server is running" for any lookup failure. |
| 34 | `04fbf5c5c`, `47e0001f4` | A view-only client stayed in raw mode until a key was pressed; cursor colour was left changed on exit. |

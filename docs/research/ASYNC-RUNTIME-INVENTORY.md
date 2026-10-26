# Async runtime inventory

Read-only inventory, 2026-10-22, base `53e472fd4`. Question: which workspace
crates use `tokio`, `futures`, `futures-util` and `async-stream`, which API
each uses, and what `rusty_tokio` lacks before those crates can switch. Issue
#686. No code changes. This is the go/no-go gap list. The pilot migration and
follow-up tickets come after it.

## 0. Verdict

- **`tokio` → `rusty_tokio`: go, in three tiers.** The compiler resolves every
  `tokio` path the workspace names (94) in `rusty_tokio`, except three (gaps
  G2 and G3). 24 of the 94 resolve only under a different module (G1). 37
  crates can move now (tier A). 22 more need gaps closed first (tier B). 27 cannot move while
  a third-party dependency they use runs on tokio (tier C, G9).
- **`futures` / `futures-util` / `async-stream`: no-go until G7 is built.**
  The ticket names `rusty_stream` as their replacement, but it is not one.
  `rusty_stream` is a durable log server ("Single-node durable log for
  RustyMill"): it has no `Stream` trait, no combinators and no `stream!`
  macro. Nothing in the workspace provides that API. Owner decision
  (2026-10-22): rename the log server to `rusty_journal` (#895) and build a
  new `rusty_stream` from scratch for this API (#896).

## 1. Totals

86 workspace crates depend directly on at least one of the four (normal, dev
or build). Frozen Nexus is outside the workspace and not counted.

| Dependency | Crates | Dev only | In-house replacement |
|---|---|---|---|
| `tokio` | 81 | 24 | `rusty_tokio` |
| `futures` | 14 | 1 | none (G7) |
| `futures-util` | 10 | 1 | none (G7) |
| `async-stream` | 9 | 0 | none (G7) |
| `tokio-test` | 7 | 7 | `rusty_tokio::Runtime::block_on` (G10) |
| `tokio-util` | 4 | 1 | none (G8) |
| `futures-core` | 2 | 0 | none (G7) |
| `tokio-stream` | 1 | 0 | none (G7) |

The ticket counted 80 / 14 / 9 / 9. This count resolves workspace
inheritance through `cargo metadata` and counts one entry per package. 60
crates use `tokio` and nothing else from the list.

Most used API, by crates that use it (uses). `use` imports and inline paths
are both counted.

| API | Crates | Uses |
|---|---|---|
| `#[tokio::test]` / `#[tokio::main]` | 63 / 15 | 2,459 / 23 |
| `tokio::spawn` | 33 | 232 |
| `tokio::time::timeout` / `sleep` | 32 / 27 | 96 / 206 |
| `tokio::net::TcpListener::bind` | 16 | 144 |
| `tokio::io::AsyncWriteExt` / `AsyncReadExt` | 16 / 15 | 27 / 24 |
| `tokio::sync::mpsc` / `oneshot` / `RwLock` / `Mutex` | 13 / 9 / 12 / 7 | 22 / 15 / 13 / 10 |
| `tokio::select!` | 12 | 23 |
| `tokio::task::spawn_blocking` | 11 | 21 |
| `tokio::runtime::Runtime` / `Handle` / `Builder` | 9 / 8 / 5 | 17 / 12 / 15 |
| `futures::StreamExt` / `futures_util::StreamExt` | 8 / 10 | 14 / 46 |
| `async_stream::try_stream!` / `stream!` | 7 / 2 | 11 / 3 |
| `futures::stream::BoxStream` / `future::BoxFuture` | 6 / 3 | 10 / 3 |
| `tokio_util::sync::CancellationToken` | 4 | 20 |

`#[tokio::test(...)]` / `#[tokio::main(...)]` arguments: `flavor =
"multi_thread"` 142, `flavor = "current_thread"` 43, `start_paused = true` 20,
`worker_threads = N` 118. 25 `select!` sites.

## 2. Gaps

Each row is a difference between the external API in use and the in-house
crate. "Close" says whether to close it in-house. Crate counts for G1-G9
come from the Gaps column of section 4.

| # | Gap | Evidence | Crates | Close |
|---|---|---|---|---|
| G1 | Same items, different modules: `tokio::net::*` and `net::tcp::*` are `rusty_tokio::io::*`; `tokio::runtime::*` is at the crate root; `signal::unix::*` is `signal::*`; `io::unix::AsyncFd` is `io::AsyncFd`; `task::spawn_blocking` is `rusty_tokio::spawn_blocking` | probe (method); `rusty_tokio/src/lib.rs:324`, `src/io/mod.rs:72-107` | 39 | yes: add `net` and `runtime` re-export modules plus `task::spawn_blocking`, so a migration renames the crate and nothing else |
| G2 | `sync::Mutex::const_new` missing (static mutexes) | probe: E0599 | 1 | yes: closed in #997 (#898) |
| G3 | `time::Instant` is private; `rusty_tokio` uses `std::time::Instant`, so `pause`/`advance` cannot move a caller's `Instant::now()` | probe: E0603; `src/time/mod.rs:21` | 1 | yes: a public `Instant` read from the runtime clock; closed in #997 (#898) |
| G4 | `#[rusty_tokio::main/test]` rejects `flavor = "multi_thread"` and `"current_thread"` (185 attributes); it accepts only `worker_threads` and `flavor = "thread_per_core"`. Both runtimes exist (`Builder::new_multi_thread`, `new_current_thread`) | `rusty_tokio-macros/src/args.rs:30-49` | 16 | yes: map each flavor to its `Builder`; closed in #997 (#899) |
| G5 | `start_paused = true` rejected (20 attributes). `time::pause` exists. The macro docs still say "no pausable clock (issue #56)", which is out of date | `rusty_tokio-macros/src/lib.rs:34-38`; `src/time/mod.rs:300` | 1 | yes: closed in #997 (#899) |
| G6 | `select!` supports 2-5 arms, irrefutable patterns only, no `else`, no `, if` guard, no `biased;`. 9 of 25 sites exceed it: refutable pattern 6, `biased` 4, guard 2, `else` 1, more than 5 arms 1 | `src/macros.rs:8-29` | 8 | yes: rewrite as a proc macro in `rusty_tokio-macros`, which already depends on `syn`, so the module's "new dependency" reason no longer applies |
| G7 | No in-house `Stream` trait, combinators (`StreamExt::next/map/filter/then/boxed/buffered`, `TryStreamExt`), `BoxStream`/`BoxFuture`, `stream::iter/once/unfold/select_all`, `future::join_all/ready`, `channel::mpsc::unbounded` or `stream!`/`try_stream!` | `rusty_stream/Cargo.toml:5`, `src/lib.rs:18-36` | 24 | yes: a new `rusty_stream` with its own `Stream` trait, combinators and macros (#896). An optional `futures-core` interop feature adapts in both directions, the way `rusty_tokio` keeps `futures-io` and `bytes` (`rusty_tokio/Cargo.toml:30-48`), because axum SSE, reqwest `bytes_stream` and tonic all speak `futures_core::Stream` |
| G8 | `tokio_util::sync::CancellationToken` (20 uses) | per-crate table | 4 | yes: small, on `sync::Notify`; closed in #997 (#901) |
| G9 | Third-party dependencies that need a tokio runtime: `reqwest` 10, `axum` 9, `aisdk` 4, `sqlx` 4, `hyper` 3, `tower`/`tower-http`/`hyper-util`/`tonic` 2 each, `tokio-postgres`, `h2`, `redis`, `aws-sdk-s3`, `meilisearch-sdk` | `Cargo.lock`, direct normal dependencies only | 27 | no: these crates keep `tokio` until the framework is replaced (#687), following the precedent of D4 in `RUSTY-MCP-SOVEREIGNTY.md`. Two of them are softer, see below |
| G10 | `tokio_test::block_on` appears only in doctests (7 `rusty_adk` crates) | `adk-*/src/lib.rs` doc comments | 7 | yes: `rusty_tokio::Runtime::new()?.block_on` |
| G11 | Public API exposes `tokio` or `futures` types, so dependents migrate together: `rusty_lsp`, `rusty_request`, `rusty-db-core`, `rusty_term`, `rusty_homelab_mcp`, `rk-app` | single-line `pub` signatures | 6 | no gap: ordering only, see section 3. `rusty_http`, `rusty_request`, `rusty_tls` and `rusty_h2` already ship a `rusty-tokio` feature beside `tokio` |
| G13 | Channel endpoints (`mpsc`, `oneshot`, `broadcast`, `watch` senders and receivers) lacked `Debug`, which tokio implements; a struct deriving `Debug` over one failed to build. Found by the pilot | `rk-constrain` `ApprovalRequest` | 1 | yes: closed in the pilot PR |
| G12 | Method-level surface (builder methods, `*Ext` methods, `select!` arm bodies) is not probed | method | — | checked crate by crate by compiling; the pilot is the first check |

The two softer G9 cases are `sha256`, which reaches tokio only through its
async helper (`remind_me_core`, `remind_me_remote`), and `winit`/
`accesskit_winit`, which reach it through the platform accessibility bus
(`rusty_term`). Tokio runs inside those crates and no tokio type crosses into
workspace code. Replacing `sha256` with the in-house hash (#688) removes one
edge but does not by itself move either `remind_me` crate out of tier C:
`remind_me_remote` still depends on axum, and `remind_me_core` on the AWS SDK
(optional `cloud-backup`, `rt-tokio`).

## 3. Order

1. **Pilot `rk-constrain`** (tier A, #686 deliverable 3; done). It has no
   workspace async dependencies and no G1 paths, and its `spawn`,
   `sync::mpsc`, `sync::oneshot`, `JoinHandle` and 21 tests exercise the
   scheduler, the channels and the test macro. Its public API exposed tokio
   channel types to `rk-agui`, `rk-app` and the desktop shell. It now exports
   `approval_channel()` and the `ApprovalTx`/`ApprovalRx`/`ApprovalResponder`
   aliases, so callers never name a runtime crate. `rusty_tokio`'s channels
   need no `rusty_tokio` runtime, and those dependents still drain them from
   tokio. The pilot surfaced G13.
2. Gap tickets, independent of each other and of the pilot: #897 (G1, first:
   39 crates need it, and it turns the rest of tier A into a crate rename),
   #898 (G2, G3), #899 (G4, G5), #900 (G6), #901 (G8). #895 renames the log
   server to `rusty_journal`, then #896 builds `rusty_stream` (G7).
3. Providers: #903 moves `rusty_http`, `rusty_request`, `rusty_h2` and
   `rusty_hyper` (done). Their shipped builds already used `rusty_tokio` or
   no runtime, with `tokio` an opt-in feature. What remained were the
   `h2`-oracle interop tests in `rusty_h2` and `rusty_hyper`, which now run
   the oracle on `rusty_tokio` too, keeping `tokio` only for the `tokio::io`
   traits `h2` names. `tokio` stays a dev-dependency in two cases: those
   trait definitions, and the tests of `rusty_http`'s and `rusty_request`'s
   own opt-in `tokio` feature. Running the oracle on `rusty_tokio` found a
   parity bug: `TcpStream::poll_shutdown` failed with `NotConnected` where
   tokio succeeds.
4. Families, each blocked as its ticket says:

| Ticket | Family | Crates | After |
|---|---|---|---|
| #904 | rusty_search | 10 | #903 |
| #905 | homelab | 4 | #903, #897, #899 |
| #906 | rusty_mcp | 5 | #903, #897, #899 |
| #907 | rusty_tailscale | 9 | #903, #897, #899, #900 |
| #908 | rusty_adk (includes G10) | 12 | #896, #897, #900 |
| #909 | rusty_db (streams done; runtime held by sqlx, G9) | 2 | #896 |
| #910 | rusty_provider | 2 | #896, #897, #899 |
| #911 | small crates (rusty_key, skillopt, lsp, tonic, misc) | 10 | #903, #897, #899, #896 |
| #912 | tier C tracking | 27 | #687, #688 |

#911 tier A (2026-10-25): `skillopt-core`, `agentgateway-agui` and
`rusty_mill_board` are on `rusty_tokio`, and `rk-mcp`'s tests are. `rk-mcp`
keeps `tokio::sync::Mutex` for its connection lock, which is held across
remote awaits, until #1064 makes `rusty_tokio`'s mutex cancellation-safe. Four tier A rows stay on tokio because
of the runtime boundary: `rusty_tokio`'s timers, I/O and processes need a
`rusty_tokio` runtime, and these crates run on a tokio one. `rk-compose`'s
checks (`tokio::process`, `time::timeout`) run inside `rk-app` (tier C);
`skillopt-cli` drives `skillopt-model`'s `reqwest`; `agentgateway-tls` is the
tokio-to-`rusty_tokio` I/O bridge for `agentgateway`; and
`rusty-hister-model`'s tests drive `sqlx` through `rusty-db`. Each moves with
the tier C crate that holds it.
`rusty_tonic` (tier B) builds its tonic-oracle test streams with
`rusty_stream` through `Compat` instead of `futures-util`. `tokio` stays a
dev-dependency only to host the tonic oracle server.

## 4. Per-crate table

Deps: `t` tokio, `f` futures, `fu` futures-util, `as` async-stream, `tu`
tokio-util, `tt` tokio-test, `fc` futures-core, `ts` tokio-stream; `(dev)`
means dev-only. Files: source files that name one of these crates. API: the six
most used items with use counts. Tokio-bound deps: G9. Uses: workspace crates
in this table that it depends on (normal dependencies). Tier: **A** can move
now (G1 at most), **B** needs the listed gaps closed, **C** blocked by G9.

Method: `cargo metadata --no-deps` gives direct dependencies. A regex scan of
each package's `.rs` files (nested packages skipped, `//` comments and
doctests excluded) expands `use` trees and inline paths. Each distinct `tokio`
path (94) was compiled against `rusty_tokio` (features `bytes`,
`futures-io-compat`, `tracing`, `thread-per-core`) as `use rusty_tokio::…;`,
or as `let _ = rusty_tokio::Type::f;` for associated functions. Paths that
failed were compiled again under the G1 module mapping. Errors that were only
type inference (E0282/E0283) count as resolved. Dependency closure is walked
through `Cargo.lock`. The counts are not a vetted audit.

| Crate | Path | Deps | Files | API used (top 6, count) | Tokio-bound deps | Uses (in-house, async) | Gaps | Tier |
|---|---|---|---|---|---|---|---|---|
| `agentgateway` | `apps/rusty_agent_gateway/crates/agentgateway` | t tu | 21 | t::test×292, t::spawn×27, t::net::TcpListener::bind×26, tu::sync::CancellationToken×16, t::time::sleep×8, t::select!×3, +11 | axum, hyper, hyper-util, tower | agentgateway-agui, agentgateway-auth, agentgateway-llm, agentgateway-mcp, agentgateway-proxy, agentgateway-tls, rusty-mcp, rusty_mcp_axum, rusty_mcp_server | G1 G4 G6 G8 G9 | C |
| `agentgateway-agui` | `apps/rusty_agent_gateway/crates/agentgateway-agui` | t(dev) | 1 | t::test×2 | — | — | — | A |
| `agentgateway-auth` | `apps/rusty_agent_gateway/crates/agentgateway-auth` | t(dev) | 1 | t::test×12 | reqwest | rusty-mcp | G9 | C |
| `agentgateway-llm` | `apps/rusty_agent_gateway/crates/agentgateway-llm` | as fu t | 2 | t::test×2, fu::StreamExt×1, t::time::sleep×1, as::stream!×1 | hyper, reqwest | — | G7 G9 | C |
| `agentgateway-mcp` | `apps/rusty_agent_gateway/crates/agentgateway-mcp` | t tu(dev) | 3 | t::test×28, t::task::spawn_blocking×2, t::time::timeout×2, t::runtime::Handle×1, t::net::TcpListener×1, tu::sync::CancellationToken×1, +2 | tonic, tonic-prost | rusty-mcp, rusty_mcp_client_native, rusty_mcp_server | G1 G8 G9 | C |
| `agentgateway-proxy` | `apps/rusty_agent_gateway/crates/agentgateway-proxy` | t | 1 | t::time::sleep×1, t::time::timeout×1 | hyper, hyper-util | — | G9 | C |
| `agentgateway-tls` | `apps/rusty_agent_gateway/crates/agentgateway-tls` | t | 3 | t::test×3, t::io::duplex×3, t::net::TcpStream×2, t::io::AsyncRead×2, t::io::AsyncWrite×2, t::time::timeout×1, +7 | — | — | G1 | A |
| `rusty-hister-model` | `apps/rusty_hister/crates/rusty-hister-model` | t(dev) | 7 | t::test×120 | — | rusty-db | — | A |
| `rusty_homelab_mcp` | `apps/rusty_homelab_mcp` | t | 7 | t::test×23, t::runtime::Handle×5, t::runtime::Handle::current×2, t::main×1, t::task::spawn_blocking×1 | — | rusty_fedora, rusty_http, rusty_mcp_server, rusty_opnsense, rusty_proxmox | G1 G4 | B |
| `rk-agui` | `apps/rusty_key/crates/agui` | t | 2 | t::runtime::Handle×1, t::sync::mpsc×1, t::sync::oneshot×1, t::task::JoinHandle×1, t::spawn×1, t::select!×1, +1 | aisdk | rk-app, rk-constrain | G1 G6 G9 | C |
| `rk-app` | `apps/rusty_key/crates/app` | f t | 17 | t::test×55, t::fs::create_dir_all×26, t::io::split×10, t::spawn×7, t::io::duplex×5, t::select!×3, +27 | aisdk, axum, tower, tower-http | rk-compose, rk-constrain, rk-feed, rk-kernel, rk-mcp, rusty_mcp_server | G1 G4 G6 G7 G9 | C |
| `rk-compose` | `apps/rusty_key/crates/compose` | t | 1 | t::test×2, t::process::Command::new×1, t::time::timeout×1 | — | — | — | A |
| `rk-constrain` | `apps/rusty_key/crates/constrain` | t | 2 | t::test×8, t::sync::mpsc×1, t::sync::oneshot×1, t::task::JoinHandle×1, t::spawn×1 | — | — | — | A |
| `rk-feed` | `apps/rusty_key/crates/feed` | t | 14 | t::test×37, t::fs::read_to_string×2, t::fs::write×2, t::spawn×2, t::process::Command×2, t::sync::mpsc::UnboundedSender×1, +7 | aisdk, reqwest | rk-constrain | G9 | C |
| `rk-kernel` | `apps/rusty_key/crates/kernel` | f t | 3 | t::test×6, f::StreamExt×1, t::task::block_in_place×1, t::runtime::Handle::current×1, f::Stream×1, f::stream::once×1 | aisdk | rk-constrain | G1 G4 G7 G9 | C |
| `rk-mcp` | `apps/rusty_key/crates/mcp` | t | 3 | t::test×4, t::sync::Mutex×1 | — | rk-constrain, rk-feed, rusty-mcp-client | — | A |
| `rusty_mill_board` | `apps/rusty_mill_board` | t | 2 | t::signal::ctrl_c×2, t::runtime::Builder::new_current_thread×1, t::signal::unix::signal×1, t::signal::unix::SignalKind::terminate×1, t::select!×1, t::runtime::Builder×1, +1 | — | rusty_http, rusty_mcp_server, rusty_request | G1 | A |
| `rp-core` | `apps/rusty_provider/crates/core` | f | 1 | f::stream::BoxStream×1 | — | — | G7 | B |
| `rp-mcp` | `apps/rusty_provider/crates/mcp` | t | 4 | t::test×9, t::time::sleep×4, t::spawn×3, t::runtime::Handle×2, t::time::timeout×2, t::sync::RwLock×1, +1 | — | rp-core, rp-router, rusty-mcp-client, rusty_mcp_server | G1 G4 | B |
| `rp-providers` | `apps/rusty_provider/crates/providers` | fu t(dev) | 7 | t::test×110, fu::StreamExt×6 | reqwest | rp-core | G7 G9 | C |
| `rp-router` | `apps/rusty_provider/crates/router` | f t | 5 | t::test×156, t::time::sleep×33, t::spawn×7, t::task::spawn_blocking×4, f::stream::StreamExt×1, f::stream×1, +2 | reqwest, tokio-postgres, tokio-postgres-rustls | rp-core, rp-providers | G1 G5 G7 G9 | C |
| `rp-server` | `apps/rusty_provider/crates/server` | fu t | 6 | t::test×133, t::time::timeout×3, fu::StreamExt×2, t::sync::Semaphore::new×2, t::net::TcpListener::bind×2, t::spawn×2, +5 | axum, reqwest, tower-http | rp-core, rp-mcp, rp-router, rusty_mcp_axum, rusty_mcp_server | G1 G4 G7 G9 | C |
| `remind_me_core` | `apps/rusty_remind_me/crates/remind_me_core` | t | 1 | t::runtime::Builder::new_current_thread×1 | aws-config, aws-sdk-s3, sha256 | — | G1 G9 | C |
| `remind_me_remote` | `apps/rusty_remind_me/crates/remind_me_remote` | t | 4 | t::test×29, t::net::TcpListener::bind×4, t::spawn×3, t::runtime::Builder::new_multi_thread×1, t::sync::Mutex×1, t::sync::Mutex::const_new×1, +2 | axum, sha256 | remind_me_core, rusty_mcp_axum, rusty_mcp_server | G1 G2 G9 | C |
| `skillopt-cli` | `apps/rusty_skillopt/crates/skillopt-cli` | t | 1 | t::main×1 | — | skillopt-core, skillopt-model | — | A |
| `skillopt-core` | `apps/rusty_skillopt/crates/skillopt-core` | t(dev) | 1 | t::test×3 | — | — | — | A |
| `skillopt-model` | `apps/rusty_skillopt/crates/skillopt-model` | t | 11 | t::test×21, t::io::AsyncWriteExt×2, t::process::Command::new×2, t::time::timeout×2 | reqwest | skillopt-core | G9 | C |
| `ts-cli` | `apps/rusty_tailscale/crates/ts-cli` | t | 2 | t::main×1, t::net::UnixStream×1 | — | rusty_http | G1 G4 | B |
| `ts-control` | `apps/rusty_tailscale/crates/ts-control` | t | 6 | t::test×10, t::time::timeout×5, t::io::AsyncRead×4, t::io::AsyncWrite×4, t::io::AsyncReadExt×4, t::spawn×4, +11 | h2 | rusty_http | G1 G9 | C |
| `ts-daemon` | `apps/rusty_tailscale/crates/ts-daemon` | t | 1 | t::signal::ctrl_c×2, t::signal::unix::SignalKind×1, t::signal::unix::signal×1, t::main×1, t::spawn×1, t::select!×1, +1 | — | ts-engine, ts-localapi | G1 G4 | B |
| `ts-derp` | `apps/rusty_tailscale/crates/ts-derp` | t | 4 | t::test×9, t::spawn×3, t::io::AsyncRead×2, t::io::AsyncReadExt×2, t::io::AsyncWriteExt×2, t::time::timeout×2, +9 | — | rusty_http | G1 | A |
| `ts-engine` | `apps/rusty_tailscale/crates/ts-engine` | t | 1 | t::spawn×2, t::sync::mpsc×1, t::sync::oneshot×1, t::time::timeout×1, t::time::interval×1, t::time::MissedTickBehavior::Delay×1, +2 | — | ts-control, ts-derp, ts-magicsock, ts-tun | G6 | B |
| `ts-localapi` | `apps/rusty_tailscale/crates/ts-localapi` | t | 2 | t::spawn×4, t::net::UnixStream×2, t::test×2, t::net::UnixListener×1, t::io::AsyncReadExt×1, t::io::AsyncWriteExt×1, +2 | — | rusty_http | G1 | A |
| `ts-magicsock` | `apps/rusty_tailscale/crates/ts-magicsock` | t | 1 | t::io::unix::AsyncFd×1, t::net::lookup_host×1, t::time::timeout×1, t::net::UdpSocket×1, t::net::UdpSocket::from_std×1, t::test×1 | — | ts-derp, ts-stun | G1 | A |
| `ts-net` | `apps/rusty_tailscale/crates/ts-net` | t | 4 | t::spawn×3, t::time::timeout×3, t::sync::mpsc×2, t::sync::oneshot×2, t::io::AsyncReadExt×2, t::time::sleep×2, +8 | — | ts-engine | G4 G6 | B |
| `ts-stun` | `apps/rusty_tailscale/crates/ts-stun` | t(dev) | 1 | t::net::UdpSocket×1, t::test×1, t::spawn×1, t::time::timeout×1 | — | — | G1 | A |
| `ts-tun` | `apps/rusty_tailscale/crates/ts-tun` | t | 1 | t::io::unix::AsyncFd×1 | — | — | G1 | A |
| `rusty_fedora` | `libs/homelab/rusty_fedora` | t(dev) | 1 | t::test×10 | — | rusty_request | — | A |
| `rusty_opnsense` | `libs/homelab/rusty_opnsense` | t(dev) | 1 | t::test×30 | — | rusty_request | — | A |
| `rusty_proxmox` | `libs/homelab/rusty_proxmox` | t(dev) | 1 | t::test×23 | — | rusty_request | — | A |
| `rusty_h2` | `libs/net/rusty_h2` | t(dev) | 1 | t::net::TcpListener::bind×5, t::spawn×5, t::runtime::Runtime×4, t::time::sleep×2, t::runtime::Builder::new_multi_thread×1, t::time::timeout×1, +1 | — | — | G1 | A |
| `rusty_http` | `libs/net/rusty_http` | t | 1 | t::test×24, t::io::AsyncRead×1, t::io::AsyncReadExt×1, t::io::AsyncWrite×1, t::io::AsyncWriteExt×1, t::io::ReadBuf×1, +2 | — | — | — | A |
| `rusty_hyper` | `libs/net/rusty_hyper` | t(dev) | 1 | t::spawn×3, t::runtime::Runtime×2, t::net::TcpListener::bind×1, t::runtime::Builder::new_multi_thread×1, t::net::TcpStream::connect×1 | — | rusty_h2, rusty_http | G1 | A |
| `rusty_request` | `libs/net/rusty_request` | t | 6 | t::io::AsyncRead×4, t::test×3, t::io::AsyncWrite×2, t::runtime::Handle::try_current×1, t::time::timeout×1, t::time::sleep×1, +6 | — | rusty_http | G1 | A |
| `rusty_a2a` | `libs/protocol/rusty_a2a` | as fc fu t ts tu | 44 | t::test×139, t::time::sleep×64, t::spawn×52, t::net::TcpListener::bind×41, fu::StreamExt×14, fc::Stream×9, +14 | axum, reqwest, tonic | — | G1 G7 G8 G9 | C |
| `rusty-acp` | `libs/protocol/rusty_acp` | fu t tu | 49 | t::test×219, t::spawn×51, t::net::TcpListener::bind×46, t::time::sleep×40, fu::StreamExt×15, t::time::timeout×7, +30 | axum, redis, reqwest, sqlx | — | G1 G3 G4 G6 G7 G8 G9 | C |
| `rusty_lsp` | `libs/protocol/rusty_lsp` | t | 12 | t::test×103, t::time::timeout×25, t::spawn×9, t::io::duplex×6, t::time::sleep×5, t::io::AsyncWriteExt×3, +35 | — | — | G1 G4 | B |
| `rusty-mcp` | `libs/protocol/rusty_mcp/crates/rusty-mcp` | t | 11 | t::test×84, t::spawn×14, t::time::sleep×9, t::net::TcpListener::bind×6, t::time::timeout×4, t::io::AsyncReadExt×2, +2 | — | rusty_http, rusty_request | G1 G4 | B |
| `rusty-mcp-client` | `libs/protocol/rusty_mcp/crates/rusty-mcp-client` | t(dev) | 5 | t::test×27, t::time::timeout×10, t::spawn×4, t::time::sleep×3, t::io::AsyncReadExt×2, t::io::AsyncWriteExt×2, +4 | — | rusty_mcp_client_native, rusty_request | G1 G4 | B |
| `rusty-mcp-demo` | `libs/protocol/rusty_mcp/crates/rusty-mcp-demo` | t(dev) | 1 | t::test×43, t::time::timeout×4, t::time::sleep×1 | — | rusty_mcp_server | — | A |
| `rusty_mcp_axum` | `libs/protocol/rusty_mcp_axum` | f t | 2 | t::task::spawn_blocking×7, t::test×6, t::net::TcpListener::bind×3, t::spawn×3, t::sync::mpsc×1, t::sync::oneshot×1, +1 | axum | rusty_http, rusty_mcp_server | G1 G4 G7 G9 | C |
| `rusty_mcp_client_native` | `libs/protocol/rusty_mcp_client_native` | t(dev) | 2 | t::test×8, t::time::timeout×2, t::runtime::Runtime::new×1, t::net::TcpListener::bind×1, t::spawn×1, t::time::sleep×1, +1 | — | rusty_request | G1 G4 | B |
| `rusty_mcp_server` | `libs/protocol/rusty_mcp_server` | t(dev) | 3 | t::test×4, t::time::sleep×2, t::time::timeout×2, t::process::Command×1 | — | rusty_http | G4 | B |
| `rusty_tonic` | `libs/protocol/rusty_tonic` | fu(dev) t(dev) | 2 | fu::stream::iter×3, t::runtime::Runtime×3, fu::StreamExt×2, t::runtime::Builder::new_multi_thread×2, fu::Stream×1, t::net::TcpListener::bind×1, +3 | — | rusty_h2, rusty_http, rusty_hyper | G1 G7 | B |
| `adk-a2a` | `libs/rusty_adk/crates/adk-a2a` | as f t tt(dev) | 4 | t::test×18, f::StreamExt×4, f::stream::BoxStream×2, t::net::TcpListener×2, t::spawn×2, t::time::sleep×2, +2 | — | adk-agents, adk-graph, adk-runner, rusty_a2a | G1 G6 G7 | B |
| `adk-agents` | `libs/rusty_adk/crates/adk-agents` | as f t(dev) tt(dev) | 5 | t::test×29, as::try_stream!×5, f::StreamExt×4, f::stream::BoxStream×3, f::stream::select_all×1, f::future::BoxFuture×1 | — | adk-graph, adk-models, adk-tools | G7 | B |
| `adk-agui` | `libs/rusty_adk/crates/adk-agui` | f t | 2 | f::StreamExt×1, t::runtime::Handle×1, t::runtime::Runtime×1 | — | adk-graph, adk-runner | G1 G7 | B |
| `adk-graph` | `libs/rusty_adk/crates/adk-graph` | as f t tt(dev) | 4 | t::test×21, t::sync::mpsc×2, f::future::BoxFuture×1, f::StreamExt×1, f::stream::BoxStream×1, as::try_stream!×1, +1 | — | — | G7 | B |
| `adk-macros` | `libs/rusty_adk/crates/adk-macros` | t(dev) | 1 | t::test×7 | — | — | — | A |
| `adk-mcp` | `libs/rusty_adk/crates/adk-mcp` | t | 7 | t::test×7, t::spawn×6, t::io::duplex×4, t::io::AsyncReadExt×3, t::io::AsyncWriteExt×3, t::io::AsyncRead×2, +18 | axum | adk-tools, rusty-mcp-client, rusty_mcp_axum, rusty_mcp_server | G1 G9 | C |
| `adk-models` | `libs/rusty_adk/crates/adk-models` | f t tt(dev) | 2 | t::test×6, f::stream×2, f::stream::BoxStream×2, f::stream::StreamExt×1 | reqwest | — | G7 G9 | C |
| `adk-runner` | `libs/rusty_adk/crates/adk-runner` | as f t(dev) tt(dev) | 1 | t::test×8, f::stream::BoxStream×1, f::StreamExt×1, as::try_stream!×1 | — | adk-agents, adk-graph | G7 | B |
| `adk-sessions` | `libs/rusty_adk/crates/adk-sessions` | t tt(dev) | 5 | t::test×52, t::task::spawn_blocking×1 | — | — | G1 | A |
| `adk-tools` | `libs/rusty_adk/crates/adk-tools` | f | 1 | f::future::BoxFuture×1 | — | — | G7 | B |
| `rusty-adk` | `libs/rusty_adk/crates/rusty-adk` | f(dev) t(dev) tt(dev) | 1 | t::test×3, f::StreamExt×1 | — | adk-a2a, adk-agents, adk-agui, adk-graph, adk-macros, adk-mcp, adk-models, adk-runner, adk-sessions, adk-tools | G7 | B |
| `a2a-agent-server` | `libs/rusty_adk/examples/a2a-agent-server` | t | 1 | t::main×1, t::net::TcpListener::bind×1 | axum | adk-a2a, rusty-adk | G1 G9 | C |
| `agui-agent-server` | `libs/rusty_adk/examples/agui-agent-server` | t | 1 | t::runtime::Runtime::new×1 | — | rusty-adk | G1 | A |
| `mcp-tool-server` | `libs/rusty_adk/examples/mcp-tool-server` | t | 1 | t::main×1 | — | rusty-adk | — | A |
| `weather-agent` | `libs/rusty_adk/examples/weather-agent` | f t | 1 | f::StreamExt×1, t::main×1 | — | rusty-adk | G7 | B |
| `rusty-db-core` | `libs/storage/rusty_db/crates/rusty-db-core` | t | 1 | t::time::timeout×1 (`with_timeout`; every caller runs sqlx on tokio) | — | — | G9 (via drivers) | C; streams on `rusty_stream` (#909) |
| `rusty-db-mysql` | `libs/storage/rusty_db/crates/rusty-db-mysql` | — | 0 | — (streams on `rusty_stream`, sqlx bridged by `Compat`, #909) | sqlx | rusty-db-core | G9 | C |
| `rusty-db-postgres` | `libs/storage/rusty_db/crates/rusty-db-postgres` | — | 0 | — (streams on `rusty_stream`, sqlx bridged by `Compat`, #909) | sqlx | rusty-db-core | G9 | C |
| `rusty-db-sqlite` | `libs/storage/rusty_db/crates/rusty-db-sqlite` | — | 0 | — (streams on `rusty_stream`, sqlx bridged by `Compat`, #909) | sqlx | rusty-db-core | G9 | C |
| `rusty-db` | `libs/storage/rusty_db/rusty_db` | t(dev) | 97 | t::test×391, t::time::sleep×13, t::task::spawn_local×7, t::task::LocalSet×7, t::time::timeout×3, t::sync::oneshot×3, +2 | sqlx (via drivers) | rusty-db-core, rusty-db-mysql, rusty-db-postgres, rusty-db-sqlite | G9 | C: every test drives sqlx, which panics without a tokio runtime (#909) |
| `rusty-search` | `libs/storage/rusty_search/crates/rusty-search` | t(dev) | 1 | t::main×1 | — | rusty-search-algolia, rusty-search-azure-search, rusty-search-elasticsearch, rusty-search-meilisearch, rusty-search-memory, rusty-search-opensearch, rusty-search-solr, rusty-search-sqlite-fts5, rusty-search-tantivy | — | A |
| `rusty-search-algolia` | `libs/storage/rusty_search/crates/rusty-search-algolia` | t | 1 | t::test×13, t::sync::RwLock×1, t::time::sleep×1 | — | rusty_request | — | A |
| `rusty-search-azure-search` | `libs/storage/rusty_search/crates/rusty-search-azure-search` | t | 1 | t::test×14, t::sync::RwLock×1 | — | rusty_request | — | A |
| `rusty-search-cloud` | `libs/storage/rusty_search/crates/rusty-search-cloud` | t(dev) | 1 | t::test×1 | — | — | — | A |
| `rusty-search-elasticsearch` | `libs/storage/rusty_search/crates/rusty-search-elasticsearch` | t | 1 | t::test×15, t::sync::RwLock×1 | — | rusty_request | — | A |
| `rusty-search-meilisearch` | `libs/storage/rusty_search/crates/rusty-search-meilisearch` | t | 1 | t::test×8, t::sync::RwLock×1 | meilisearch-sdk | — | G9 | C |
| `rusty-search-memory` | `libs/storage/rusty_search/crates/rusty-search-memory` | t | 1 | t::test×10, t::sync::RwLock×1 | — | — | — | A |
| `rusty-search-opensearch` | `libs/storage/rusty_search/crates/rusty-search-opensearch` | t(dev) | 1 | t::test×6 | — | rusty-search-elasticsearch, rusty_request | — | A |
| `rusty-search-solr` | `libs/storage/rusty_search/crates/rusty-search-solr` | t | 1 | t::test×11, t::sync::RwLock×1 | — | rusty_request | — | A |
| `rusty-search-sqlite-fts5` | `libs/storage/rusty_search/crates/rusty-search-sqlite-fts5` | t | 1 | t::test×19, t::sync::RwLock×1 | — | — | — | A |
| `rusty-search-tantivy` | `libs/storage/rusty_search/crates/rusty-search-tantivy` | t | 1 | t::test×15, t::sync::RwLock×1 | — | — | — | A |
| `rusty_term` | `libs/ui/rusty_term` | t | 2 | t::spawn×5, t::time::sleep×4, t::select!×3, t::runtime::Builder::new_multi_thread×2, t::pin!×2, t::time::interval×2, +15 | accesskit_winit, winit | — | G1 G6 G9 | C |

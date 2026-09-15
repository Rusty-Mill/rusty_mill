# gap-analysis.md — rustils_async vs. rustils

**Run date:** 2026-08-12
**Reference:** `rustils` pinned at `83ab7a9ed2c4ffe90df9acdd6688cc74b7aed694` (the same rev `rustils_async`'s workspace `Cargo.toml` already pins for its `platform`/`platform-mock`/`platform-linux` git dependencies — kept pinned for this run rather than following upstream).

## Rerun — 2026-08-12 (after issues #4/#5/#6 merged)

Re-audited `AsyncSpawner`/`AsyncChild` (now including the merged `wait_any`,
`take_stdin`/`take_stdout`/`take_stderr`, `wait_job`/`try_wait_job`) against
the current `rustils::platform::process::{Spawner, Child}` trait definitions.

- Checked whether `rustils` drifted since the pinned rev: `origin/main` is
  now at `9d3ab36` (was `83ab7a9`), but `git diff 83ab7a9..9d3ab36 --
  crates/platform/src/process.rs crates/platform-linux/src/sys/spawn.rs`
  is empty — the intervening commits are an unrelated Windows-socket
  (`WSAENOBUFS`) fix. The process domain is unchanged; the existing pin
  stays valid, no re-pin needed.
- Found one gap the original pass missed: **`Spawner::is_zombie`** has no
  `AsyncSpawner` counterpart. Filed as issue #10 (`parity-gap`,
  `platform:linux`) and added to the table below.
- `Child::as_any_mut` (a downcast hook `Spawner::wait_any`'s *native*
  override uses internally to reach a backend's own OS handles) is **not**
  filed as a gap: `AsyncSpawner`'s `wait_any` already gets genuine
  multiplexing through `AsyncChild::ready()` being a real trait method
  (issue #4), so there is no downcast-based native override to support in
  the first place — this is a sync-side implementation detail with no
  async-side counterpart to be missing.

## Rerun — 2026-08-12 (after issue #10/PR #11 merged)

Re-checked for drift and re-enumerated the full `Spawner`/`Child` trait
surface against `AsyncSpawner`/`AsyncChild` symbol-for-symbol, rather than
re-verifying only the previously-filed rows (the same discipline that
caught `is_zombie` last time).

- `rustils`' `origin/main` is still at `9d3ab36` — no commits landed
  upstream since the prior rerun, so there is nothing new to diff and the
  pin stays valid unchanged.
- Full symbol enumeration, this time both directions:
  - `Spawner`: `spawn`, `resolve`, `wait_any`, `adopt`, `is_alive`,
    `is_zombie` — all six present on `AsyncSpawner` (the last two as
    synchronous pass-throughs, per `RM-DEV-ASYNC-0001`).
  - `Child`: `wait`, `id`, `kill_tree`, `kill_single`, `try_wait`,
    `wait_job`, `try_wait_job`, `take_stdin`, `take_stdout`, `take_stderr`
    — all nine present on `AsyncChild` (plus `ready`, the async-only
    multiplexing primitive `wait_any` is built on, with no sync
    counterpart to diff against by construction). `as_any_mut` stays
    excluded for the reason already given above.
  - No other in-scope symbols: `Command`/`Stdio`/`EnvSpec`/`GroupSpec`/
    `ExitStatus`/`Signal`/`PlatformError` are shared types via the pinned
    git dependency (no gap possible by construction); `GroupHandle`
    already matches (`kill_tree`/`kill_single`); fs/net/Windows/BSD stay
    out of scope per the roadmap, unchanged.
- **Result: zero gaps found.** `AsyncSpawner`/`AsyncChild` now cover the
  `Spawner`/`Child` surface completely for the one domain both repos are
  committed to. No issues filed, no PR needed beyond this doc update — the
  `parity-loop` stop condition ("no open `parity-gap` issues remain")
  holds, confirmed on a second consecutive rerun rather than assumed
  stable from the first.

## Step 0 — scope, settled from the target's own roadmap

`rustils_async` already carries a hand-curated scope doc, so this run audits
against it rather than generating a competing scope from a mechanical diff:

- **README.md** — "Reserved, not built" table: `platform-async-windows`,
  `platform-async-bsd`, and async fs/net domains are explicitly reserved,
  gated on "a real consumer." `threading` is explicitly scoped to what the
  Rusty-Mill AKB's own threading capability doc treats as settled, not its
  still-draft wait/atomics/scheduling surface.
- **`docs/adr/0001-native-async-rustils.md`** — records that this repo starts
  with the `process` domain only, and that further expansion should trace to
  a real caller, the same test `coreutils-async`/`arun` satisfies for
  `process` itself.

Per parity-loop's own step 0 rule ("if one exists, it *is* the definition of
parity for this run"), this run does **not** re-open fs/net/Windows/BSD as
candidate gaps — that would contradict, not audit, the existing roadmap.
Threading is also left alone this round for the same reason (its upstream
scope in `rustils` isn't the async question anyway — `rustils` has no
threading module of its own to diff against).

**What "parity" means for this run:** within the one domain both repos are
already committed to (`process`), does `platform-async`'s `AsyncSpawner`/
`AsyncChild` cover the same functional surface as `rustils::platform::process`'s
`Spawner`/`Child`? This is the "no comparable surface to diff" path (`cargo
public-api` isn't useful here — the async trait method names/signatures
necessarily differ from their sync counterparts even where the capability is
the same) — assessed by reading both trait definitions directly.

`Command`, `Stdio`, `EnvSpec`, `GroupSpec`, `ExitStatus`, `Signal`,
`PlatformError` are the literal same types on both sides (shared via the
pinned git dependency) — no gap possible there by construction.

## Step 1 — gap table

| Symbol | Category | Source | Platforms | Reference | Existing RustyMill impl | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `wait_any` (async multi-child wait) | fn (new, on `AsyncSpawner`) | roadmap | linux | `platform::process::Spawner::wait_any` / free fn `platform::process::wait_any` | not checked — RustyMill org repos (`rush`, `rusty_tokio`) are outside this session's attached owner tier; `add_repo` refused the cross-tier add | no | M | The actual point of async: wait for whichever of N children finishes first without blocking a thread per child. Sync side already has a portable default + Linux pidfd-multiplexed override; async side currently has no multi-child wait at all — only one-at-a-time via `AsyncChild::wait`. Implement as an `EpollReactor`-driven future analogous to the single-child path, registering all pidfds at once. |
| `take_stdin` / `take_stdout` / `take_stderr` | fn (new, on `AsyncChild`) | roadmap | linux | `platform::process::Child::{take_stdin,take_stdout,take_stderr}` | not checked, same reason as above | no | S | `Command`'s `Stdio::Pipe` variant is already shared/reusable, but `AsyncChild` has no way to retrieve the parent-side pipe handles at all today — `arun` never exercises `Stdio::Pipe` because there's nothing to take. Returns `Box<dyn platform::fs::File>` same as the sync side (fs remains sync — no fs async surface exists or is in scope this round); this is plumbing, not new async machinery. |
| `wait_job` / `try_wait_job` | fn (new, on `AsyncChild`) | roadmap | linux | `platform::process::Child::{wait_job,try_wait_job}` | not checked, same reason as above | no | M | Unix job-control (`WUNTRACED`/`WCONTINUED`) — stop/continue, not just terminate. Sync side supports it for `rush`'s foreground-job tracking (per `rustils`' own doc comments). Async equivalent needs the reactor to distinguish "terminated" from "stopped/continued" instead of always treating pidfd-readable as terminal — real design work, not a thin wrapper, hence M not S. |
| `is_zombie` | fn (new, on `AsyncSpawner`) | roadmap (rerun) | linux | `platform::process::Spawner::is_zombie` | not checked, same reason as above | no | S | Already non-blocking on the sync side (`/proc/<pid>/stat`'s state field, `Unsupported` on Windows per divergence 015) — `RM-DEV-ASYNC-0001` says this stays sync, same reasoning already applied to `is_alive`/`adopt`/`resolve`. Pure pass-through addition to `AsyncSpawner`, mirroring `is_alive`'s existing shape exactly. Missed in the original pass (2026-08-12 initial run); caught on rerun by re-diffing the full trait surface rather than just the three previously-filed gaps. |

## Step 1 — deliberately excluded (in-scope check performed, not a gap for this run)

- `GroupHandle` (`adopt`) — already covered; `AsyncSpawner::adopt` exists and
  matches the sync signature exactly (mechanism-only pass-through).
- `fs`, `net`, `pty`, `security`, `term`, `tun`, `events` domains — out of
  scope per the roadmap (README's "Reserved, not built" table); not
  re-litigated here.
- Windows/BSD backends for the process domain itself — out of scope per the
  same table; the three gaps above are trait-level additions to
  `platform-async` (portable) but only get a real (non-`unimplemented!`)
  implementation on `platform-async-linux`, matching how this repo already
  handles "reserved" platforms (row retained, not stubbed).

## Notes on the "Breaking?" column

All three gaps are **new methods added to existing traits** (`AsyncSpawner`,
`AsyncChild`), not changes to any existing method's signature or behavior.
Per parity-loop's rule, "Breaking? yes" is reserved for a fix that touches an
*existing* public signature — none of these do. In the strict Rust-semver
sense, adding a required trait method is still API-breaking for an external
implementor, but `platform-async`'s traits have exactly two implementors
today (`platform-async-mock`, `platform-async-linux`), both in this same
workspace, both updated in the same PR as each trait change, and the crate
is unpublished (`publish = false`). Marked `no` on that basis — flagged here
explicitly rather than silently assumed.

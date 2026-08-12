# gap-analysis.md — rustils_async vs. rustils

**Run date:** 2026-08-12
**Reference:** `rustils` pinned at `83ab7a9ed2c4ffe90df9acdd6688cc74b7aed694` (the same rev `rustils_async`'s workspace `Cargo.toml` already pins for its `platform`/`platform-mock`/`platform-linux` git dependencies — kept pinned for this run rather than following upstream).

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

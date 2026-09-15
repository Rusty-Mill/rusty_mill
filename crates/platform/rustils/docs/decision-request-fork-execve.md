# Decision needed: fork/execve vs posix_spawn (Layer 1 Linux spawn)

Not a design document — a decision request, the same shape
`docs/decision-request-msys-parity.md` used. This is the single item
`docs/convergence-roadmap.md` has carried as "Parked: fork/execve vs
posix_spawn" and `docs/architecture.md` has carried as its one "Open
item recorded" since the roadmap opened — dug into for the first time
here, rather than left as a two-sentence pointer.

## Outcome

**Option 3 decided and landed, 2026-08-06** — `memfd_create` wired
into `platform::fs` as `AnonymousFile::create_memfd`
(`docs/extraction-map.md`'s D11 entry, `docs/behavior/fs.md`'s own
section), independent of everything else in this document.

**Options 1 vs. 2 decided the same day: option 1 — stay on
`posix_spawn` indefinitely.** `docs/architecture.md`'s target picture
(the one active disagreement this document opened against) is updated
to match; `docs/convergence-roadmap.md`'s own parked entry is closed,
not just annotated. Reasoning, weighted in the order it was actually
weighed: `posix_spawn` already delivers the safety property that
matters (every allocation in the parent, before the call — the
async-signal-safe critical region gone by construction, not managed);
PTY hosting (D13) already made this identical call once, independently,
under real pressure, and landed on the same answer
(`POSIX_SPAWN_SETSID` over `fork`+`TIOCSCTTY`); and this document's own
finding — that "adopting rusty_libc's fork+execve" describes donor
material that doesn't exist, making option 2 a two-repo commitment to a
new hazard class — tipped it decisively, with no named consumer
forcing that cost. Revisit only if a real consumer or a recovered
rationale beyond Track P's own general "no glibc in the spawn path"
aspiration ever surfaces; nothing here expires on its own.

## Where this stands today

- Layer 1 Linux spawn is entirely `posix_spawn`
  (`platform-linux/src/sys/spawn.rs`) — confirmed, zero raw `fork`/
  `clone`/`execve` calls anywhere in this crate, on either the `libc`
  or `track-p` configuration.
- The owner-confirmed architecture diagram shows raw fork/execve on
  Layer 1 Linux instead — `docs/architecture.md`'s own words: *"The
  one active disagreement between this picture and current code."*
- `docs/convergence-roadmap.md` frames resolving it toward raw as
  "adopting rusty_libc's fork+execve" and names `memfd_create` (D11)
  as the prerequisite that makes a raw `clone(SIGCHLD)` fork sound.

## What "adopting rusty_libc's fork+execve" actually requires — the finding that changes this

Checked rusty_libc's real, current source — the exact pinned
dependency `platform-linux/Cargo.toml` already carries behind the
`track-p` feature (`rev dfa4e8c...`, MSRV 1.88, off on the workspace's
1.75-floor CI leg per that Cargo.toml's own comment) — rather than
trusting the roadmap's framing at face value:

- **`memfd_create` already exists** (`rusty_libc/src/fd.rs:572`) — the
  D11 prerequisite this repo's own parked note names is sitting there
  unused, not missing. Wiring it into `platform::fs` is a small,
  independent, low-risk slice regardless of which way the larger
  decision goes.
- **The x86_64 `SA_RESTORER` signal-return trampoline** — D4's own
  "wrong = crash on first delivered signal" hazard — is already
  solved in `rusty_libc/src/signal.rs`, hand-written, naked-fn, in
  place today.
- **Real POSIX process primitives are present**: `setpgid`/`kill`/
  `killpg`/`setsid`/`getpgid`/`getsid`/`pidfd_open` all exist, wrapped,
  real syscalls, in `rusty_libc/src/process.rs`.
- **`fork`, `execve`, and `clone` do not exist anywhere in rusty_libc.**
  Grepped the whole crate for `SYS_clone`/`SYS_fork`/`SYS_execve`/
  `SYS_vfork` and every plausible function name in `process.rs` —
  nothing. Every function there is a getter/setter/signal-delivery
  primitive; none of them start a new process.

**This means "adopting rusty_libc's fork+execve" describes donor
material that doesn't exist yet, not a wire-it-in job.** Every other
Track P adoption this repo has done so far (`pidfd_open`,
`getdents64`, the raw-syscall `fdio::read`/`write` slice — see
extraction-map.md's own "suggested sequence" step 3) followed the same
shape: rusty_libc already had the wrapper, rustils called it behind
`track-p`. Going raw here breaks that pattern — it is a **two-repo
project**, not a rustils-side flag flip. New fork/clone/execve syscall
wrappers, with their own async-signal-safety story rewritten to not
outsource to glibc (D4's whole ~25-syscall analysis: the
`SA_RESTORER` trampoline, kernel-vs-glibc `termios` layout,
aarch64's removed syscalls, the raw-errno-vs-glibc-TLS contract — all
things `posix_spawn` currently lets glibc worry about), would need to
land in rusty_libc first, as that repo's own PR and review, before
rustils could consume anything.

## Why current code (posix_spawn) exists, and what it's already avoided reopening

`sys/spawn.rs`'s own module doc: `posix_spawn` is preferred
specifically because it "removes the async-signal-safe critical region
from this crate entirely — every allocation... happens before the
call, in the parent" — the fix, by construction, for the v1 scaffold's
B-1/B-2 bug class. D11's `memfd_create` material is cited as the
*reason* the fork-vs-malloc-lock deadlock class (another thread
holding the allocator lock at fork time) becomes safely avoidable if
raw fork is ever adopted — a thread-free here-doc removes the helper
thread that made "single-threaded at every fork point" a fragile
invariant to maintain in the first place.

**PTY hosting (D13) already faced this exact question and declined
raw fork, explicitly.** `docs/design-discussion-pty.md`'s own
"posix_spawn substitute for fork+TIOCSCTTY" section: shh's donor code
uses `fork`+`pre_exec`+`TIOCSCTTY`; this crate reached the identical
outcome (child ends up session leader with the pty slave as its
controlling terminal) through `posix_spawn`'s own `POSIX_SPAWN_SETSID`
flag plus a file-actions pathname open instead — specifically because
"reopening that gap for one more slice isn't this issue's call to
make unilaterally." This decision has already been deferred once, on
the record, by a slice that could have forced it and chose not to.

## What raw fork/execve would actually buy, if pursued

The roadmap states *that* the target picture wants raw fork/execve; it
doesn't record *why*, beyond the architecture diagram citation. Worth
being honest that this document has no recovered rationale beyond
that pointer. Plausible reasons, none verified against an owner
statement:

- Removing the libc/glibc dependency entirely from the spawn path —
  Track P's own stated end goal (a from-scratch syscall floor, D4/D-2).
- More control over the exact sequence between fork and exec than
  `posix_spawn_file_actions`'s declarative model offers — a real, if
  narrow, expressiveness gap `pre_exec`-shaped C libraries lean on.
- Matching `rush`'s own donor shape more closely, should `rush`
  interactive (Phase 5) become a real convergence target — though
  `pre_exec`-set async-signal-safety is *rush's* problem to solve as a
  shell; this repo's own `posix_spawn` choice was made specifically to
  not inherit that hazard itself.

## Options

1. **Stay on `posix_spawn` indefinitely.** Update `docs/architecture.md`'s
   "target picture" to match reality instead of carrying an
   acknowledged disagreement — the diagram becomes descriptive of the
   decision made, not aspirational. Lowest cost; already the
   safer-by-construction choice PTY's own design pass independently
   arrived at when it faced the same question. **Chosen — see Outcome
   above.**
2. **Commission the rusty_libc work first**, as its own gated project
   in that repo (fork/clone/execve wrappers, the async-signal-safety
   story rewritten from scratch), *then* revisit adopting it here —
   matching the two-repo shape this document found, not the one-repo
   shape the roadmap's current wording implies. **Not chosen.**
3. **Land the small, independent win regardless**: wire `memfd_create`
   into `platform::fs` now — it's already sitting in the pinned
   dependency, D11's own text already names it as a `Dir`-adjacent
   primitive — without committing either way on the larger fork/execve
   question. Decouples the one piece that's genuinely ready from the
   one that isn't. **Chosen — see Outcome above.**

## Open questions for the owner, not decided here

1. Is there a recovered rationale for the target picture wanting raw
   fork/execve beyond the architecture diagram's own citation — a real
   requirement this document doesn't have access to — or should the
   diagram itself be revisited given what `posix_spawn` has already
   avoided reopening (D13's own precedent)?
2. If option 2: does commissioning new rusty_libc work fit that repo's
   own roadmap and ownership model? This document has no visibility
   into rusty_libc's own priorities or whether its maintainer(s) would
   take on fork/exec's own hazard class.
3. Is option 3 (`memfd_create` alone) worth landing independently of
   how 1 vs 2 resolves — a small `Dir`-adjacent Fs primitive with no
   real dependency on the fork/execve question beyond historical
   association in D11's own text?

## What this document decided

All three options are resolved — see Outcome above. Nothing in this
family is still pending an owner call as of 2026-08-06.

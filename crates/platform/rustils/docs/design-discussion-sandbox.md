# Design discussion — Security surface, sandbox policy (Phase 6 item 3)

Not a decision record — but see **Outcome** below, added once the owner actually
made the calls this document's open questions raised. This is the RFC-level
discussion `docs/convergence-roadmap.md`'s Phase 6 entry says to expect before
writing any code for the largest, most design-sensitive item in the Security
surface. It exists to surface what the two named donors (nexus, shh) actually
built — verified against their source, not their own docs' framing — and the real
open questions that follow from that, for the owner to decide before this becomes
a `platform::security` trait.

## Outcome

The owner's call on question 5's fork (build now vs. hold for a confirmed
consumer): **build the confinement half now, regardless of a confirmed live
consumer** — accepting the speculative-build risk explicitly, since nexus's own
implementation is the closest thing this repo has to a validated design to mirror.
Landed as `platform::security::Sandbox` (`confine_filesystem` via raw Landlock
syscalls, `block_inet_sockets` via a hand-written seccomp-BPF filter, both Linux/
`x86_64`-only for now) — see `docs/behavior/security.md` for the full contract.

shh's privilege-separation pattern (question 3) stayed explicitly out of scope:
it doesn't fit `platform::process`'s current shape (no raw `fork`/`setuid`/
`prctl`/`socketpair` exposed anywhere in this crate), and question 5's own
reasoning for splitting the two halves applied — nothing forced them to land
together. `CredentialStore` (Phase 6 item 2) stayed held at the time, per the
same session's separate finding that nexus's `CredentialVault` has no live gap
to converge on (see its own PR/commit history for that check).

**Update, 2026-07-23**: `CredentialStore` has since landed too
(rustils#76/#77/#78), the same kind of owner's-explicit-call this document's
`Sandbox` outcome already was — still no confirmed live consumer, built anyway.
See `docs/behavior/security.md`'s `CredentialStore` section for the full
contract; question 5's text below is left as the historical record of the
reasoning at the time, not amended in place.

## What the donors actually built

### nexus: process confinement (Landlock + seccomp, Linux only)

`nexus-security/src/os_sandbox.rs` narrows what an already-about-to-run process can
touch:

- **Filesystem** (Landlock ABI V1, kernel 5.13+): grant-only — whole-disk read, plus
  read+write on the cwd and configured writable roots. Landlock has no deny/refer
  rights at this ABI version, so it *cannot* enforce a `.git`-is-read-only carve-out;
  nexus's own docs note that gap is currently covered only by higher-layer tooling,
  not the kernel.
- **Network** (seccomp-bpf): default-allow every syscall, deny only `socket()` calls
  requesting `AF_INET`/`AF_INET6`/`AF_PACKET`. `AF_UNIX` and everything else is
  untouched — this is a narrow "no raw internet sockets" filter, not a general
  syscall allowlist. No `NO_NEW_PRIVS`, no rlimits anywhere in this path.
- **Applied via a helper-exec binary, not in-process narrowing of a running,
  multithreaded process.** `nexus-sandbox` is a small, deliberately single-threaded
  binary that confines *itself* (Landlock ruleset + seccomp filter, both of which
  survive `execve`) and then execs the real target. The doc comment is explicit about
  why: applying Landlock/seccomp from a `pre_exec` hook after `fork()` in a
  multithreaded parent is unsafe — another thread may hold the allocator lock at fork
  time, and building the ruleset/BPF program allocates. The single-threaded helper
  sidesteps the hazard rather than solving it.
- **macOS/Windows: no real confinement exists.** Both return a distinct
  `SandboxStatus::Unsupported` (not silently `FullyEnforced`, not an error) — but
  nothing forces a caller to act on that value, and nexus's own helper binary refuses
  to run at all on non-Linux. The security audit in-tree scopes explicitly to the
  iframe/WASM plugin sandbox, not this OS-process sandbox — there is no audit of the
  Linux Landlock/seccomp path, and none of the macOS/Windows gap either.
- **No disabled-mode escape hatch** the way `CredentialVault` has one (`NEXUS_NO_KEYRING=1`).
  The only "off" state is a policy value (`SandboxPolicy::DangerFullAccess`), routed
  through the same enforcement code, not an environment override.

### shh: privilege separation (fork + socketpair + credential drop, Unix only)

`src/privsep.rs` protects one specific secret — the SSH host private key — by
splitting the daemon into two processes *before* the async runtime starts (fork must
happen single-threaded):

- **Signer child**: keeps the private key, drops to an unprivileged uid/gid
  immediately (`prctl(NO_NEW_PRIVS)` → `setgid`/`initgroups`/`setuid`, in that
  order and for a documented reason — `NO_NEW_PRIVS` must be set while still
  privileged, before the uid drop, so a compromised process can't re-escalate via a
  setuid-root binary in the gap), then actively **verifies** the drop worked by
  attempting `setuid(0)` and refusing to run if it succeeds. Only after that does it
  apply `setrlimit` (`RLIMIT_NPROC=0`, `RLIMIT_NOFILE=16`).
- **Daemon parent**: drops (zeroizes) its own copy of the key, keeps one end of a
  `UnixStream::pair()` to the signer, and does all subsequent untrusted work —
  parsing pre-auth SSH protocol bytes from the network — without ever holding the key
  or root again.
- **The socketpair carries exactly one operation**: "sign this exchange hash,"
  length-framed, capped at 4096 bytes. If the signer is gone or errors, the daemon
  gets an empty signature back and the handshake fails closed — not a crash, not a
  silent bypass.
- **Linux/Unix-only** (`#![cfg(unix)]`), no macOS/Windows story, not documented as a
  deliberate scope decision — reads as "not attempted yet."
- **A separate, weaker mode** (`--sandbox`) additionally drops the *daemon's* own
  privileges the same way, but shh's own docs are explicit that this collapses all
  authenticated sessions onto one shared unprivileged account — recommended only for
  single-purpose servers, not multi-user hosts. Full per-user privilege separation
  (OpenSSH's actual model) is documented as not yet implemented.

## The core tension: these are two different problems wearing one label

Both get called "sandbox" in this codebase's own vocabulary, but they don't share a
shape:

- **nexus's need is confinement**: narrow what a process *can touch* (files, network)
  before or as it starts running arbitrary/untrusted code. The natural unit is "apply
  a policy to a process about to exec."
- **shh's need is isolation of a secret via process boundary**: keep a specific piece
  of privileged state in a process that a second, unprivileged process can ask a
  narrow question of, but never touch directly. The natural unit is "split into two
  processes early, with a hand-picked RPC between them" — a supervisor pattern built
  from lower-level primitives (fork, setuid, prctl, socketpair), not a single
  capability call.

Forcing both into one `platform::security::Sandbox` trait would couple two things
that don't need to be coupled — the same category error RFC v2 §3 already warns
against for unrelated capabilities, just less obvious here because both donors used
the word "sandbox."

## Open questions for the owner, not decided here

1. **Is nexus actually going to migrate onto a PAL confinement trait, or is this
   donor-only material?** Exactly the question that turned out to have "no" as the
   answer for `CredentialStore` last time — nexus's `os_sandbox.rs` is complete,
   working, and in production. Nothing in it references rustils or expresses a desire
   to swap onto something else. Worth confirming directly before building, not
   assuming the donor relationship implies a live consumer.
2. **Does confinement even make sense as a `dyn Trait` capability the way `Csprng`/`Net`
   do?** nexus's own design requires a dedicated single-threaded helper *binary* to
   safely apply Landlock/seccomp post-fork — not just a function call a caller
   invokes from wherever it happens to be running. If rustils took this on, does the
   PAL become responsible for building/shipping that helper binary too? That's a much
   bigger scope than "add a trait method," and no existing `platform-*` crate ships a
   binary artifact today.
3. **shh's privsep pattern doesn't fit `platform::process`'s current shape at all.**
   `platform::process` is deliberately high-level — `Command`/`Spawner`/`Child` — with
   no raw `fork`/`setuid`/`prctl`/`socketpair` exposed anywhere (checked directly:
   none of these appear in `crates/platform/src/process.rs`). Supporting shh's pattern
   would mean either (a) adding those raw primitives as a new, much lower-level slice
   of the Process surface — a real widening of what this trait exposes — or (b)
   treating privsep as a *pattern* documented for consumers to hand-build from
   existing pieces, not a PAL capability at all. These are very different amounts of
   work and very different design postures.
4. **The macOS/Windows gap is asymmetric and real, not just "not implemented yet."**
   Linux has two mature, donor-verified primitives (Landlock, seccomp). macOS's
   nearest equivalents (Seatbelt/`sandbox_init`, deprecated but functional; the App
   Sandbox entitlement system, which assumes a code-signed, packaged app) and
   Windows's (restricted tokens, AppContainer, which assumes a packaged-app identity
   too) don't obviously fit an arbitrary CLI-launched process the way Landlock/seccomp
   do. `Unsupported` as a return value is precedented elsewhere in this codebase, but
   nexus's own experience shows the real risk: nothing stops a caller from ignoring
   it and running unconfined without realizing it. If rustils ships this, what's the
   contract — does a consumer have to explicitly branch on `Unsupported`, or does the
   trait itself refuse to degrade silently (closer to how nexus's own seccomp path
   hard-refuses on failure rather than degrading)?
5. **If both halves do move forward, do they ship as one Phase-6-item-3 PR, or as two
   separate, independently-consumer-gated slices** (confinement gated on nexus
   actually converging; privsep gated on shh actually converging, and only after
   deciding whether it's a trait or a documented pattern)? Given finding #1 above,
   splitting them avoids blocking whichever one turns out to have a real consumer on
   whichever one doesn't.

## What this document does not decide

Whether to build either half, in what shape, or on what timeline. That's exactly the
RFC-level call the roadmap flagged as needing to happen before code — this is the
input to that call, not the call itself.

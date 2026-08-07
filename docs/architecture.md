# Target Architecture — the four layers and the ecosystem convergence map

**Status:** Adopted 2026-07-19 (RFC v2 Amendment A3). This document records
the target-state picture the owner confirmed; the RFC's consumer gate (§3)
governs *when* each future surface gets built, this document records *where*
everything belongs when it does. For *in what order* — the phased
migration/convergence sequencing across this repo and the parallel
tools — see [`docs/convergence-roadmap.md`](convergence-roadmap.md).
A rendered version of this picture, including the 2026-07-21
beside-the-PAL shelf additions recorded below, is
[`docs/architecture.svg`](architecture.svg).

```
┌──────────────────────────────────────────────────────────────┐
│  LAYER 3 — Application                                       │
│  rush · coreutils · rusty_tail · rusty_naner · rusty_lsp     │
│  SHH · rusty_rdp · rusty_whisper · rusty_llama · nexus       │
│  (beside, not on top: rusty_lines · rusty_regx · planned:    │
│   rusty_tls · rusty_http · rusty_wire · rusty_ansder ·       │
│   rusty_json — see Layer 3 notes)                            │
├──────────────────────────────────────────────────────────────┤
│  LAYER 2 — Platform Abstraction Layer (platform crate)       │
│  today: Fs · Process · Events · Net (TCP/Unix/UDP, done) ·   │
│         Security (Csprng, Sandbox confinement — 2 of 3       │
│         slices; CredentialStore held, no live consumer) ·    │
│         errors · parity · mock                               │
│  gated: Terminal · Windowing · Registry/Config               │
├──────────────────────────────────────────────────────────────┤
│  LAYER 1 — OS Implementation (all unsafe lives here)         │
│  platform-linux (libc floor · rusty_libc track-p)            │
│  platform-windows (windows-sys floor · rusty_win32 track-w)  │
├──────────────────────────────────────────────────────────────┤
│  LAYER 0 — OS Kernel                                         │
│  Linux kernel ABI            ·           Windows NT kernel   │
└──────────────────────────────────────────────────────────────┘
```

## Layer 0 — OS Kernel

What the machine actually offers; nothing here is ours.

| Linux kernel ABI | Windows NT kernel |
|---|---|
| syscall tables (x86_64 / aarch64 — no fork/poll on aarch64) | ntdll syscall gate (NtCreateFile, NTSTATUS) |
| file descriptors (openat, pipe2, statx, getdents64) | object manager (HANDLEs, NT namespace) |
| signals + sigreturn (rt_sigaction, SA_RESTORER) | I/O manager (files, pipes, handle inheritance) |
| processes + groups (clone/execve, wait4, kill(-pgid)) | processes + Job Objects (kill-on-close) |
| pidfd + poll/ppoll | wait dispatcher (WaitForMultipleObjects, 64-cap) |
| vDSO (clock_gettime fast path) | console subsystem (ctrl events) |

## Layer 1 — OS Implementation

Per-OS personality, honestly expressed. All `unsafe` in the workspace is
confined here (`sys/` module trees, CI-enforced), below a curated `ffi/`
surface where every admitted symbol is listed and justified.

- **platform-linux** — `sys/{fdio,spawn,signals}` over two floors: the
  `libc` crate (default, D-2) and **rusty_libc** raw syscalls behind
  `track-p` (D-12). Migrated call-by-call: read/write, the openat family
  (statx), pipe2/poll, kill/wait4/rt_sigaction incl. the SA_RESTORER
  trampoline. `posix_spawn` stays permanently on both configurations —
  a decided design choice, not a gap (see "Decided" below); remaining
  open items are getdents64 and pidfd_open (upstream additions to
  rusty_libc).
- **platform-windows** — `sys/{nt,proc,fileio,csignals}` + `winargv` over
  two floors: **windows-sys** (default, D-1) and **rusty_win32**'s
  hand-written `extern "system"` declarations behind `track-w` (D-15),
  migrated call-by-call starting with `sys::fileio::read`/`write`.
  rusty_win32 was already this backend's extraction donor — its typed-
  handle, `wait_any` and console patterns were mined into `sys/` (see the
  extraction map) — and D-15 turns that one-way flow into a real,
  rev-pinned dependency, the same move D-12 made for rusty_libc. Note the
  asymmetry with Track P, recorded rather than glossed: Track P descends a
  tier (raw syscalls under libc), Track W does not — Windows exposes no
  supported tier below a documented DLL export, so `track-w` swaps the
  binding's *provenance*, not its depth, and D-1 stands unchanged. The
  donor crate still runs independently; the `winargv` handback still flows
  the other way.

## Layer 2 — Platform Abstraction Layer

The `platform` crate: portable traits and types, `#![forbid(unsafe_code)]`,
made real by the parity regime (behavior specs + parity suite on both OS
legs + the divergence registry, D-13) and by `platform-mock` as the third
backend.

**Built:** `Fs` (capability Dir/File, byte OsStr boundary D-11), `Process`
(Command/Spawner/Child, decoded ExitStatus B-5, groups/kill_tree, pipes),
`Events` (deferred SignalSource D6, multiplexed wait_any — the §5.6
reactor), `Net` (all three D16 slices: TCP connect/listen/accept/
set_nodelay, Unix domain sockets with mode+stale-cleanup bind, UDP
datagram — D16's full four-consumer survey now landed), the two-axis
error model.

**Gated future surfaces** (each unparks only when its named consumer
arrives, §3):

| Surface | Forcing consumer(s) | Notes |
|---|---|---|
| Terminal | rusty_lines' host, shh, rush interactive, rusty_naner (console-acquisition facet) | five independent donor hand-rolls (D9); **rusty_term is the design oracle** — trait built fresh in Layer 2, rusty_term converges by swapping its backend internals |
| PTY (Process×Terminal) | a future emulator/mux consumer | donors: shh openpty, rusty_term ConPTY (D13); divergences: ConPTY vs openpty, pollable fd vs thread bridge |
| Tun / virtual link | rusty_tail | /dev/net/tun ioctls vs wintun (D14) |
| Windowing | nexus front-ends | Tauri-mediated in nexus, so it converges last and thinnest; rusty_rdp's "display" is wire-encoding, not OS windowing |
| Registry / Config | nexus (ERP modules) | today hand-rolled JSON + dirs paths |
| Security — `CredentialStore` (remaining gated slice) | nexus (not yet confirmed live) | donor in hand: nexus's `CredentialVault`, checked 2026-07-20 and held — complete, working, no gap or expressed desire to migrate; revisit only if that changes |

## Layer 3 — Application

Consumers pull the PAL into shape; the PAL never speculates (§3).

- **Under contract:** `rush` (RFC §7, hoists at its Phase 2 gate).
- **Built here:** `coreutils` reference consumers (rcat, rls, rrun, rpar,
  rtee) — every PAL API proved by one of them first.
- **Parallel tools, pre-rustils, converging** (corrected by the
  2026-07-19 full-ecosystem survey): rusty_tail (a Tailscale-style mesh
  VPN — Net+Tun, NOT a log follower), rusty_naner (a Windows
  GUI-subsystem launcher/bootstrapper — Terminal console-acquisition +
  Fs staged-install + Process/winargv, NOT an editor), rusty_lsp
  (essentially converged already — zero platform crates), shh (modern
  SSH — Terminal+PTY+Net+Security), rusty_rdp (a pure wire-format codec
  — Net near-term; Windowing only via a future viewer app),
  rusty_whisper and rusty_llama (compute engines; llama adds an mmap
  model load and an optional TCP server), **nexus** (the micro-frontend
  / ERP host — donor for Security's `CredentialStore` slice and for
  Registry/Config, though `CredentialStore` was checked 2026-07-20 and
  found not to be a live forcing consumer yet — see the gated-surfaces
  table above; its hand-rolled Job Objects and Unix job control
  duplicate landed rustils work, making
  Process its cheapest first convergence).
- **Beside the PAL, not on top:** rusty_lines (line editing) and
  rusty_regx (regex) are OS-independent pure-Rust libraries; they need no
  abstraction layer and simply serve the applications. **Shelf additions
  planned 2026-07-21** (out of the TLS design research,
  `docs/design-discussion-tls.md`, and the same-day ecosystem source
  survey), governed by the shelf's own gate — extracted only where two-plus
  repos demonstrably duplicate the logic today:
  - **rusty_tls** (planned): sans-IO TLS engine wrapping rustls behind a
    consumers-never-see-rustls seam, one verify-by-default trust policy
    with a named no-verify escape hatch, sync + async adapters. Consumers:
    rusty_request (forcing), rusty_rdp (migrates its `tls.rs`), rusty_tail
    (latent). TLS stays permanently out of `Net`'s traits (D16).
  - **rusty_http** (planned, gate met): sans-IO HTTP/1.1 message layer +
    `Url` type. Six live hand-rolls today: rusty_request's
    `http1.rs`/`url.rs`/`cookie.rs` (the donor code) and four in rusty_tail
    (controlhttp, DERP client, LocalAPI client, and a hand-rolled HTTP
    *server* in ts-localapi).
  - **rusty_json** (optional, gate arguably met): rusty_request's no-serde
    `json.rs` and nexus's hand-rolled JSON config are two real
    implementations; extract when a second consumer reaches for one.
  - **rusty_wire** (planned, owner override 2026-07-21 of the initial
    "rows only" call): the endian-explicit byte-cursor `Reader`/`Writer`
    micro-crate. Donors: rdp's `cursor.rs` (287 lines, the core, taken
    near-verbatim) and shh's `wire/` (173 lines — its SSH composites stay
    a dialect above the core, never in it). Justified as the foundation
    rusty_ansder builds on, plus one fuzzed implementation of
    overrun/truncation handling instead of N.
  - **rusty_ansder** (planned, same owner override): ASN.1/DER
    definite-length TLV + general typed layer, built on rusty_wire.
    Donor: rdp's already-layered DER stack — `ber.rs` (273 lines, taken
    whole) plus the general half of `krb5/asn1.rs` (528 lines); the
    Kerberos-specific structures stay in rdp, rebuilt on top. X.509
    remains a gated row inside this crate (named consumer: rusty_tls, if
    it ever looks inside certificates); no crypto ever — it parses,
    never verifies.
  - Declined: shared crypto primitives (shh uses RustCrypto/dalek by
    choice; rdp's hand-rolls are protocol-mandated obsolete algorithms)
    and base64 (no duplication exists). rusty_provider stays off the
    shelf entirely — a parallel tokio/reqwest stack with no live gap.
  - Shelf build order: rusty_wire → rusty_ansder (dependency); rusty_tls,
    rusty_http, rusty_json are independent of each other and of those
    two.

## The convergence rule

A parallel tool converges by swapping its direct OS calls for Layer 2
traits — nothing else about it has to change. Order follows the gate: a
tool whose surfaces already exist (rusty_tail → Fs) can converge any
time; a tool needing a gated surface (rusty_naner → Terminal) is itself
the named consumer that unparks it. Convergence PRs land in the tool's
repo; new-surface PRs land here, consumer named in §3's table first.

## Decided: Layer 1 Linux spawn stays `posix_spawn`

This picture used to show raw `fork`/`execve` as the Layer 1 Linux
target, disagreeing with the code's actual `posix_spawn` choice —
recorded here as an open item and left for an owner call
(`docs/decision-request-fork-execve.md` has the full investigation).

**Decided 2026-08-06: option 1 — stay on `posix_spawn` indefinitely.**
This picture now reflects that as the settled design, not an
aspiration current code falls short of. Reasoning, in brief (full
record in the decision doc): `posix_spawn` already removes the
async-signal-safe critical region entirely — every allocation happens
in the parent, before the call — which is the safety property that
matters, not something raw fork would improve on. PTY hosting (D13)
already faced this identical fork-vs-`posix_spawn` choice under real
pressure and picked `posix_spawn` (`POSIX_SPAWN_SETSID` in place of
`fork`+`TIOCSCTTY`) specifically to avoid reopening this hazard —
established precedent, not a fresh call made in isolation. And
`docs/decision-request-fork-execve.md`'s own research found "adopting
rusty_libc's fork+execve" describes donor material that doesn't
exist yet (no `fork`/`execve`/`clone` anywhere in that crate) — going
raw would mean commissioning a new hazard class in a second repo, with
no named consumer forcing it, for a benefit that's Track P's own
aspirational "no glibc in the spawn path" rather than a concrete
requirement.

`memfd_create` (D11) — the one piece this decision's "if resolved
toward raw" branch would have needed as a prerequisite — landed
independently anyway (`docs/decision-request-fork-execve.md`'s option
3): useful on its own, not contingent on this outcome.

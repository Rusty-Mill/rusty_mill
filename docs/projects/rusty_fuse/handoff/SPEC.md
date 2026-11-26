# rusty_fuse — spec
Design chat: unknown (claude.ai chat, 2026-11-19; link not found by search) · remind-me tag `rusty_fuse`
Owner: Nano (`baileyrd`) — the person whose words count as `[stated]`
Tags: [stated] [agreed] [default] [open].
Reference (not binding): `../design.md`, condensed from the Claude Doc "Sovereign cross-platform userspace filesystem: design" rev 24, https://claude.ai/artifact/Shrtmm4Ubn68pRAh33it4D. The doc is the full text and holds the architecture diagram, which no export carried.

## Intent
Nano wants one sovereign, cross-platform userspace filesystem: a backend written once, mounted on Windows, Linux and macOS, with no third-party kernel code and no third-party userspace library by default (ADR-0002). Sovereignty-first because of NIPR/SIPR constraints. [stated]

## Scope
- In: a pure `Filesystem` port (read core) plus `Writable`; one native adapter per OS; a conformance suite; a reference passthrough backend. [agreed]
- In, order: Windows (ProjFS) first, then Linux, macOS last and low priority. [stated: D1, D4]
- Out: kernel drivers; full POSIX on in-box Windows; FreeBSD and other targets; async API; network filesystem protocols. [default]
- Deferred until a named consumer: `Xattrs`, `Locks`, cancellation, local-file leases, WinFsp adapter. [default]

## Constraints
- Name `rusty_fuse`; family under `crates/libs/storage/rusty_fuse/` [stated: D3]. Code and docs layout follow ADR-0003: libs depend only on foundation and platform.
- `fusermount3` is acceptable on Linux as a Tier T dependency with a plan to migrate away [stated: D2]; ADR-0002 Tier T needs an owner, a first-party seam and a tracked milestone (owner [open]).
- Port crate has no OS calls and no `unsafe`; all `unsafe` lives in each adapter's `sys` module. [default]
- ProjFS cleanup evicts verified clean, provider-owned cache only; local changes are preserved; never imply secure erasure [default: D16].
- Do not depend on the `fsk` crate; design reference only [default: D8].
- Nano's standing rules apply: ports-and-adapters, minimal dependencies, no async unless real I/O concurrency, tests for non-trivial logic. [stated]

## Shape
```
backend (memfs, passthrough, first real backend [open])
        │ implements
   rusty_fuse (port: Filesystem, Writable; no OS calls)
   ┌──────────┬───────────────┬───────────────┐
 _linux     _projfs         _fskit          (_winfsp, Tier A, later)
 /dev/fuse  ProjectedFSLib   Swift ext +
 + codec    via rusty_win32  Unix-socket bridge
        rusty_fuse_mount (facade, picks adapter by target OS)
```
Port, adapters, crate layout, phases: `../design.md`.

## Delivery plan [default, ordering Windows-first is stated]
0 spikes (ProjFS, `/dev/fuse`) · 1 port + Windows read-only · 2 Linux read-only · 3 Linux read-write + first fusermount3 replacement · 4 on demand · 5 macOS.

## Acceptance — first slice
- [ ] Phase 0: ProjFS hello-world mount through `rusty_win32::dynlib` on Nano's Windows machine; go or no-go recorded.
- [ ] Phase 0: `/dev/fuse` hello-world through `rusty_libc` on a Linux host; go or no-go recorded; CI feasibility known.
- [ ] Phase 0 (ProjFS): confirm cache-state names and non-forcing delete behaviour; test clean hydration versus local changes: eviction removes a clean hydrated file and leaves an edited file, a locally created file, a changed-metadata file and a locally deleted entry untouched.
- [ ] Phase 1 starts only after the first real backend is named (OPEN_QUESTIONS Q1).
- [ ] Pre-Q1 slice (D22): port read core, memfs plus conformance, and read-only passthrough on a plain folder (D23); no mount, no ProjFS. It narrows the item above for this slice only; Q1 stays open.

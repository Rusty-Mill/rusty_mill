# RFC v2 — rustils (Rust Platform Core)
## A hand-rolled, Rust-native platform personality layer for Windows and Linux

**Status:** Proposed — supersedes `docs/rfc.md`, `docs/roadmap.md`, and the architecture docs generated in the original Copilot scaffold. **Amended:** A1 (2026-07-19) re-grounds §7/§8-R2 — see the amendment note in §7 and `docs/extraction-map.md`. A2 (2026-07-19) closes every §7.3 open item and the license question — decisions D-11..D-14. A3 (2026-07-19) adopts the four-layer target architecture and ecosystem convergence map — `docs/architecture.md`.
**Date:** 2026-07-18
**Sponsor:** Nano
**Companion document:** ~~`rush-shell-plan.md` v1.2 (ADR-011)~~ — superseded by Amendment A1: the plan described an alternate rush never built; the real contract material is `docs/extraction-map.md`

---

## 0. Why this rewrite exists

The v1 scaffold was generated conversationally and inherited three incoherences that this RFC resolves rather than papers over:

1. **Purpose drift.** The originating goal was a *pure, hand-rolled, from-first-principles* rebuild of the OS interaction layer — a learning-driven project. The scaffold, however, was structured like a pragmatic delivery project, and was then evaluated (fairly) against delivery standards it never articulated. This RFC states the dual mandate explicitly and derives every rule from it.
2. **Tier contradiction.** The conversation chose syscall-level purity for Linux; the scaffold shipped `libc`. The conversation chose `windows-sys` as the deliberate Windows floor; the scaffold agreed. This RFC makes the tier choice per-platform, explicit, and staged.
3. **Scope without forcing functions.** Five domains (fs, process, window, registry, security) were declared; two were partially built; three were stubs with no consumer. Every stub was scaffold the conversation's "all of the above" pattern generated, not a decision. This RFC replaces breadth-by-default with the **consumer gate** (§3).

Nothing in v1's *direction* was wrong — windows-sys floor, typed handles, portable traits, parity testing, Windows-first CI are all kept. What changes is discipline: object-safe instance traits, a byte-oriented boundary, a sound spawn contract, unsafe containment, and a sequencing rule that ties every line of code to a named consumer.

---

## 1. Identity and Mandate

**rust-platform-core is a Rust-native OS personality layer**: a strongly-typed, safety-first API surface over the Windows NT and Linux kernels, hand-built above raw bindings, intended to become the long-term foundation ("personal `std`") for the sponsor's system-level projects.

It carries a **dual mandate**, and every design rule below traces to one or both:

- **M1 — Understanding.** The project exists partly to *learn the machine*: Win32 handle semantics, NT object lifetimes, Linux syscall behavior, process creation on both kernels, the real rules of Windows argv. Hand-rolling is therefore a feature, not an inefficiency — but M1 is only satisfied when the hand-rolled thing is *correct*. A layer that ships allocation-after-fork or unquoted argv has learned the wrong lesson; soundness requirements (§6) are non-negotiable precisely because of M1, not despite it.
- **M2 — Foundation.** The layer must be real: consumed by working software, battle-tested, and stable enough that future projects (shells, services, tooling, eventual GUI work) build on it without regret. M2 is what forbids speculative APIs and demands the consumer gate, parity discipline, and semver seriousness.

**Anti-goals** (unchanged in spirit from v1, now explicit):
- Not a Wine-style Win32 reimplementation or emulation layer; the real kernels and system DLLs remain the floor.
- Not a competitor to `std` for general audiences; interop with `std` is required (§5.1), replacement of it is not attempted.
- Not a place where APIs exist because they *might* be needed.

---

## 2. Tier Doctrine (resolving the purity contradiction)

The "how low do we go" question gets a per-platform, staged answer instead of one absolute:

| Platform | Floor (now) | Rationale | Purity track (later, optional) |
|---|---|---|---|
| Windows | `windows-sys` raw bindings | Deliberate choice from the original design (option 5): metadata-generated bindings *are* the raw layer on Windows; hand-writing `extern` blocks re-derives machine-generated facts and adds transcription bugs, teaching nothing (fails M1) while adding risk (fails M2). The hand-rolled value on Windows lives *above* the bindings: typed handles, lifetimes, quoting, the NT object model. | Selective `ntdll` usage where Win32 obscures the learning (e.g., `NtCreateFile` for handle-relative opens) — already contemplated in v1; admitted per-API with a written rationale. **Track W (Amendment A4, 2026-08-07 → D-15):** `rusty_win32`'s hand-written `extern "system"` declarations behind the off-by-default `track-w` feature, replaced call-by-call like Track P. Explicitly *not* a lower tier and so not a revision of this row's floor — Windows publishes no supported tier beneath a documented DLL export, so both configurations reach the same `kernel32!ReadFile`; what Track W swaps is the binding's provenance (hand-written and reviewed, no `windows-targets` import-lib machinery, `no_std`-capable), which is a smaller and honester claim than Track P's. See `docs/learning/003-…`. |
| Linux | `libc` via a thin, audited wrapper set | Honest acknowledgment of where the scaffold actually is (tier 2). Correctness first. | **Track P (purity):** a `#![no_std]`-compatible `sys/` module making raw syscalls (`syscall!` stubs per arch), replacing libc call-by-call behind a feature flag, each replacement landing with its own tests and a written note of what was learned. Track P is M1 work, gated to begin only after Release R2 (§8) so it never blocks M2 consumers. |

The existing hand-written `ffi/kernel32.rs`-style re-export modules are kept only as *curation points* (narrowing the imported surface), never as hand-transcribed declarations.

---

## 3. The Consumer Gate

**Rule: no API is implemented without a named, working consumer that calls it.** Domains may be *planned* — named in this RFC with their intended consumer — but planning produces a row in the table below, not code. This is the structural defense against the expansion-by-conversation dynamic that generated v1's stubs.

| Domain | Status | Gating consumer | Notes |
|---|---|---|---|
| `fs` | **Active** | `coreutils` (in-repo) | Redesigned per §5.3 (capability-style). |
| `process` | **Active** | `coreutils` now; **rush** at hoist (§7) | Redesigned per §5.4; rush is the forcing function for the reactor, groups, quoting. |
| `events` (reactor, signals) | **Contracted** | rush (arrives with the Phase-2-gate hoist) | Do not design speculatively; receive the proven shape. |
| `pty` | **Contracted** | rush Phase 5 | Same. |
| `term` (console modes) | Planned | rush Phase 5 or a TUI project | |
| `window` | Parked | none named (future GUI / micro-frontend work) | **Stub deleted** from the tree; row retained here. |
| `registry` | Parked | none named (a future Windows config consumer) | Stub deleted; row retained. |
| `security` | Parked | none named | Stub deleted; row retained. |
| `net`, `ipc`, `services` | Parked | none named (ERP/service ambitions) | From v1's future-work list; rows only. |

Deleting the parked stubs is deliberate: an empty trait in the tree is a standing invitation to fill it without a consumer; a row in this table is a recorded intention that costs nothing to keep honest. The in-repo `coreutils` crate is retained and re-scoped as the **reference consumer and M1 exercise ground** — it exists to exercise this layer and teach, and is explicitly *not* a uutils competitor; rush continues to bundle uutils per its own ADR-005.

---

## 4. Architecture

### 4.1 Layering (per backend)

```
consumers (coreutils, rush shell-host adapters, future apps)
        ↓
api/     portable, safe, zero-unsafe trait surface + concrete types
        ↓
sys/     per-OS safe wrappers; ALL unsafe lives here, each block
         carrying a documented invariant comment
        ↓
ffi/     raw bindings: windows-sys (Windows), libc → Track P (Linux)
        ↓
kernel
```

Crate layout keeps v1's workspace shape, with the layering internal to each backend crate:

```
rust-platform-core/
├── platform/           # api layer: traits, types, errors — #![forbid(unsafe_code)]
├── platform_windows/   # sys+ffi for Windows
├── platform_linux/     # sys+ffi for Linux (+ future track-P sys)
├── platform_mock/      # NEW: in-memory backend implementing every trait — the
│                       #      testability goal v1 stated and structurally forbade
├── coreutils/          # reference consumer
├── docs/               # this RFC, behavior specs, divergence registry, learning notes
└── tests/              # parity suite (§9)
```

### 4.2 CI (kept from v1, extended)

The Windows+Linux matrix from day one was v1's best practice and is retained, adding: MSRV leg, `cargo-deny`, unsafe-scope lint (unsafe outside `sys/` fails CI), Miri job for `platform_mock`-driven logic and any unsafe-adjacent code it can execute, nightly fuzz job (§9), and the parity-ratchet (a passing parity case may never regress).

### 4.3 Target state (Amendment A3)

> **Amendment A3 (2026-07-19).** The owner adopted the four-layer target
> picture — Layer 0 kernel, Layer 1 per-OS implementation, Layer 2 PAL,
> Layer 3 applications — including the placement of every ecosystem repo
> and the gated future surfaces (Terminal, Net, Windowing,
> Registry/Config, Security) with their forcing consumers named. The
> record is `docs/architecture.md`; §3's gate still governs when each
> surface unparks, and §8's R5+ row is read through that map.

---

## 5. API Design Requirements

These are the binding corrections to v1's trait surface. Each cites the failure it prevents.

### 5.1 Instance-based, object-safe, std-interoperable

All traits take `&self`/`&mut self`; no static-method traits. Consequences: backends are values, `dyn` works, `platform_mock` can be injected, and v1's own stated goal ("backends can be mocked or swapped") becomes true instead of structurally false. Every handle type implements the `std::os` interop traits (`AsFd`/`OwnedFd`, `AsHandle`/`OwnedHandle`) and conversions to/from `std::fs::File`/`std::process::Child` where semantics allow — the layer must be adoptable incrementally, not as a total buy-in island.

### 5.2 Byte-oriented boundary

No `&str` paths, arguments, or environment values anywhere in the API. Signatures take `&OsStr`/`impl AsRef<OsStr>` (or the workspace byte-string newtype shared with rush, decided at hoist time — see §7.3 open item O-1). WTF-16 conversion policy lives in exactly one Windows `sys` module and is documented once. Prevents: unrepresentable non-UTF-8 unix filenames, lossy corruption of the kind v1's `cat` performed.

### 5.3 Filesystem: capability-style, handle-relative

Replace global path functions with a `Dir` handle model: open a directory once, operate relative to it (`dir.open(rel)`, `dir.create(rel, opts)`, `dir.metadata(rel)`, `dir.read_dir()`, `dir.remove(rel)`), mapping to `openat`-family on Linux and handle-relative NT opens on Windows (a legitimate Track-P-style ntdll admission per §2). `OpenOptions`-style builders replace v1's fixed read-only/create-always calls; share-mode control is explicit on Windows. Rationale: TOCTOU hygiene; direct support for rush's `VirtualCwd` (subshell cwd without process-global state); the mock backend becomes a natural `Dir` implementation; and this is the single most instructive Windows/NT topic for M1. Evaluate `cap-std` as prior art to *learn from*, not depend on (M1).

### 5.4 Process: builder contract, groups, decoded status

```rust
Command::new(program)              // OsStr
    .args(argv)                    // argv is a LIST; joining/quoting is internal
    .cwd(dir)                      // explicit always — no inherited-cwd ambiguity
    .env(EnvSpec)                  // inherit / clean / explicit map
    .stdio(StdioSpec)              // full stdin/out/err + extra-fd/handle wiring
    .group(GroupSpec)              // Job Object / setpgid — first-class
    .spawn(&backend)? -> Child     // owned
Child::wait(self) -> ExitStatus    // consumes: double-wait unrepresentable
Child::kill_tree(&self) / kill_single(&self)
ExitStatus = Code(i32) | Signaled(Signal)   // decoded uniformly on both OSes
```

Windows quoting is a dedicated, exhaustively-tested `winargv` module (MSVCRT rules; `.bat`/`.cmd` arguments escaped under cmd rules or **refused** when unrepresentable — the BatBadBut class is this crate's highest security exposure). Linux spawning uses `posix_spawn` where sufficient; where fork+exec is required, the child section is a documented async-signal-safe critical region: all allocations (CStrings, argv arrays) performed **before** fork, `_exit` on failure, nothing else in the child. Prevents: v1's injection-by-construction spawn, dangling-CString UB, post-fork allocation UB, double-wait use-after-close, and the raw-status-word parity bug.

### 5.5 Errors: two axes, full context

`PlatformError { kind: ErrorKind, os_code: OsCode, op: &'static str, path: Option<...> }` via `thiserror`, where `OsCode` is an enum (`Errno(i32) | Win32(u32)`) — never a bare `u32` that conflates the two number spaces. Mapping tables from both OS domains into `ErrorKind` are themselves parity-tested. `impl std::error::Error` required.

### 5.6 Events and PTY (contracted shapes — do not pre-build)

The reactor (wait-any over children ∪ handles ∪ signal events ∪ timeout, with the 64-handle `WaitForMultipleObjects` limit absorbed internally) and the PTY pair (ConPTY / openpty) arrive from rush at hoist time with their semantics already proven. This RFC reserves the module names and records the requirement; it deliberately does not specify signatures rush hasn't validated yet.

---

## 6. Soundness Baseline (Release R0 blocker list)

The following defects exist in the current tree and block everything else; each is also an M1 lesson to write up in `docs/learning/`:

| ID | Defect | Fix standard |
|---|---|---|
| B-1 | `execvp_sys` builds argv from dropped `CString` temporaries (dangling pointers, UB) | Own all CStrings in a Vec that outlives the call; add a Miri-checked construction test |
| B-2 | Child allocates and calls `std::process::exit` after `fork` | Pre-fork allocation; `_exit`; documented critical region per §5.4 |
| B-3 | Windows spawn joins args with spaces, zero quoting (injection) | `winargv` per §5.4, fuzzed against an argv-echo oracle binary on Windows CI |
| B-4 | `wait(&handle)` closes the handle through a shared reference (double-wait UAF); RAII wrappers exist but are unused in the process path | Owned-handle types used end-to-end; `wait(self)` |
| B-5 | Linux returns raw `waitpid` status; Windows returns decoded code | Uniform decoded `ExitStatus` (§5.4); parity test pins it |

Standing policy thereafter: `#![forbid(unsafe_code)]` in `platform`, `platform_mock`, `coreutils`; unsafe confined to `sys/` with per-block invariant comments; CI-enforced.

---

## 7. The rush Contract (connectable)

> **Amendment A1 (2026-07-19).** This section was written against
> `rush-shell-plan.md`, a planning document for an *alternate* rush that
> was never built; the real [`baileyrd/rush`](https://github.com/baileyrd/rush)
> predates this RFC, took its own path, and — with its satellite crates
> `rusty_libc`, `rusty_win32`, `rusty_lines`, `rusty_regx` — has already
> built and CI-proven the mechanisms §7.2 contracted to receive. There is
> no ADR-011 counterparty and no Phase 2 gate to wait on. The division of
> labor in §7.1 stands (it matches rush's actual code structure), but the
> sequencing in §7.2 is re-grounded: **R2 is an extraction project this
> repo may start at any time**, porting semantics and tests from the
> donors catalogued in `docs/extraction-map.md`, re-floored on §2's tier
> doctrine, with rush's own suites as the conformance oracle. O-1..O-3
> remain open and are decided during extraction. §7.2's original text is
> retained below as the record of what was superseded.

This section was mirrored as **ADR-011** in `rush-shell-plan.md` v1.2 — see Amendment A1 above: that document described an alternate rush that was never built, and this section is now read through the amendment.

### 7.1 Division of labor

rust-platform-core owns **mechanisms**: spawn/quoting, process groups & kill-tree, the reactor, pipes with inheritance control, PTY, signal event sources, capability-fs, decoded exit status. rush's `shell-host-*` adapters own **shell policy**: PATHEXT-vs-execbit resolution, shebang emulation, SignalSource→trap mapping, case-sensitivity queries feeding glob semantics, `/dev/null`→`NUL` mapping. The test: if a hypothetical second process-orchestrating consumer would want it unchanged, it's mechanism (here); if it encodes a shell opinion, it's policy (rush).

### 7.2 Sequencing — the hoist

1. rush Phases 0–2 build `winargv`, spawn, reactor, and groups **inside `shell-host-win`/`-unix`**, directly on windows-sys/rustix. rust-platform-core does not attempt these in parallel (no divergent twins).
2. At the **rush Phase 2 gate** — spawn surface proven, before Phase 4 multiplies call sites — the mechanisms are hoisted down into this repo, refactored to §5's API standards, and rush's adapters become thin consumers. This is Release **R2** here.
3. PTY hoists at rush Phase 5. Track P (§2) may begin only after R2.
4. Between rush phase gates, this repo's consumed API surface is **frozen** (semver-honored); evolution happens at gates. This bounds the two-repo debugging cost of a solo maintainer.

### 7.3 Open items — all decided 2026-07-19 (Amendment A2)

- **O-1 — decided: `OsStr`-only.** The entire extraction (Dir, Command,
  resolve, six consumers, both parity suites) shipped on
  `OsStr`/`OsString` without a boundary-level byte-manipulation need
  ever appearing; the one place raw units matter (`winargv`) correctly
  uses `&[u16]`, which a byte newtype would not have served. §5.1's
  std-interop works *because* `OsStr` is std's own boundary type. Byte
  manipulation is consumer policy (rush's expansion is `String`-based
  and converts at its own edges). Revisit only if a real byte-indexed
  boundary consumer appears. → D-11.
- **O-2 — decided: rusty_libc becomes rustils's Track P backend now.**
  R2/R3 having landed, the Track P gate is open; rather than staying
  rush-side until R4 "matures", `rusty_libc` is adopted as the
  raw-syscall floor behind the `track-p` feature (pinned git dependency,
  one source of truth — not a vendored fork), replaced call-by-call per
  §2's original plan with tests and learning notes per replacement.
  → D-12.
- **O-3 — decided: two registries, cross-referenced** (the recorded
  default, confirmed): mechanism divergences live in this repo's
  `docs/divergences.md` (001–003 so far), shell-behavior divergences in
  rush's docs, each citing the other where related. Revisit only if
  entries duplicate heavily; none do. → D-13.
- **License — decided: MIT**, matching every sibling crate so code flows
  both directions (extraction in, `winargv` handback out) under one
  license. → D-14.

---

## 8. Roadmap (actionable, consumer-gated)

| Release | Content | Gate / exit criterion | Consumer forcing it |
|---|---|---|---|
| **R0 — Sound** | Fix B-1..B-5; delete parked stubs; land `platform_mock`; CI extensions (§4.2); this RFC replaces v1 docs | All B-fixes landed with tests; unsafe-scope lint green; parity suite runs (small) on both OS legs | integrity of everything after |
| **R1 — Redesigned core** | §5.1/5.2/5.3/5.5 across fs+process; `Command` builder (without reactor); coreutils ported to the new surface; behavior specs written for every portable API | coreutils passes its suite against `platform_windows`, `platform_linux`, **and `platform_mock`**; parity ratchet armed | coreutils |
| **R2 — The extraction** (Amendment A1) | Port winargv (with cmd-rules/refusal — closes rush's BatBadBut gap), spawn internals, groups, wait-any from the donors in `docs/extraction-map.md`; O-1..O-3 decided | ported mechanisms green under this repo's parity suite on both OSes, with rush's suites as the oracle; rush adoption is the follow-on, not the gate | coreutils now; rush on adoption |
| **R3 — Interactive mechanisms** | PTY hoist; term/console-mode module if rush Phase 5 demands it | rush interactive smoke test green through this layer | rush (Phase 5) |
| **R4 — Track P** | Linux raw-syscall `sys` behind a feature, call-by-call, each with tests + a learning note | libc-free build of coreutils passes the same suite | M1 (explicitly) |
| **R5+ — New domains** | window / registry / security / net / ipc — each unparked **only** when its consumer exists and is named by amending §3's table | per-domain | future projects |

No calendar dates: this project is gated by rush's phase gates and by M1 appetite, and pretending otherwise would repeat the v1 scaffold's confidence-without-basis. The ordering and gates are the commitments.

---

## 9. Testing & Parity (the product discipline)

Promoted from v1's single parity test to the governing regime:

1. **Behavior specs**: every portable API ships `docs/behavior/<api>.md` stating semantics per OS *before* the parity tests are written — the spec is what parity is measured against.
2. **Parity suite**: the same test source run on both CI legs asserting identical observable behavior; the exit-status decoding bug (B-5) becomes its permanent regression sentinel. Ratchet: once passing, never regresses.
3. **Divergence registry**: numbered entries where the OS forces a difference (same mechanism as rush's; cross-referenced per O-3). A divergence may cite only an OS limitation, never implementation convenience.
4. **Mock-first unit tests**: `platform_mock` carries the bulk of consumer-logic coverage (this is what §5.1 buys).
5. **Fuzzing**: `winargv` against the argv-echo oracle (release gate); path-normalization fuzz once §5.3 lands.
6. **Miri**: on everything it can execute; mandatory for the B-1/B-2 fix regressions.
7. **Learning notes** (`docs/learning/`): each Track-P replacement and each B-fix lands with a short write-up — M1's deliverable is understanding, and unwritten understanding evaporates.

---

## 10. Decision Log Seeded by This RFC

- **D-1** Windows floor is windows-sys; hand-rolled value begins above the bindings. (§2)
- **D-2** Linux floor is libc now; raw syscalls are Track P, post-R2, feature-gated. (§2)
- **D-3** Consumer gate governs all domain expansion; parked stubs deleted. (§3)
- **D-4** Instance/object-safe/std-interop trait surface; mock backend is a first-class crate. (§5.1)
- **D-5** Byte-oriented API boundary. (§5.2)
- **D-6** Capability-style fs. (§5.3)
- **D-7** Builder-based process API; decoded ExitStatus; refuse-unrepresentable batch quoting; async-signal-safe fork discipline. (§5.4)
- **D-8** Two-axis error model. (§5.5)
- **D-9** Mechanism/policy split with rush; hoist at rush Phase 2 gate; frozen-between-gates API. (§7)
- **D-10** Parity-as-product regime. (§9)
- **D-11** `OsStr`-only byte boundary — O-1 closed. (§7.3, A2)
- **D-12** `rusty_libc` adopted as the Track P backend behind the `track-p` feature, pinned by rev — O-2 closed. (§7.3, A2)
- **D-13** Two cross-referenced divergence registries — O-3 closed. (§7.3, A2)
- **D-14** License is MIT, family-wide consistency. (§7.3, A2)
- **D-15** `rusty_win32` adopted as the Track W backend behind the `track-w` feature, pinned by rev — the Windows counterpart of D-12. Does **not** supersede D-1: windows-sys stays the default floor, because Track W is a provenance swap between peers, not a descent to a lower tier (Windows publishes no supported tier below a documented DLL export — both floors call the same `kernel32!ReadFile`). See `docs/learning/003-…`. (§2)

---

*Approval of this RFC authorizes Release R0 only. R1 requires R0's gate; everything downstream is consumer-gated.*

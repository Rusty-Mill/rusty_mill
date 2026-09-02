# Rusty Mill — Architecture

Rusty Mill provides an MSYS2-like capability, cross-platform rather than
Windows-only. It follows MSYS2's layer model, with one deliberate inversion
at the centre of it.

**MSYS2 makes Windows look like POSIX to unmodified C source you did not
write.** That is why it must emulate `fork()` — the source calls `fork()` and
you cannot edit it.

**Rusty Mill makes Windows, Linux, and macOS look alike to new Rust code you
are writing now.** Nobody is forcing our hand, so the hard POSIX-emulation
surfaces are declined outright rather than approximated.

That is not a smaller MSYS2. It is a different product in the same slot:
compatibility-for-recompiled-source versus compatibility-for-new-source. The
consequence is that we inherit MSYS2's *shape* and almost none of its
*design*, and cannot run any of its packages, ever.

## The layers

| Layer | MSYS2 analog | Owns | Status |
|---|---|---|---|
| **0 — Native OS** | Windows NT, Linux, macOS kernels | Nothing of ours. No Rusty Mill policy. | n/a |
| **0a — Host binding substrate** | (no analog) | Target-specific external bindings to Layer 0: `windows-sys`, `libc`, and narrow donors such as `rusty_win32` / `rusty_libc`. **Not a Rusty Mill layer and not a policy surface** — implementation inputs to an adapter. | External |
| **1 — Runtime** | `msys-2.0.dll` | One small versioned portable contract plus per-host adapters. Filesystem roots, paths, process, terminal, environment, locking, standard directories, capabilities, errors — and the conformance evidence for all of it. | **Built** — first vertical slice |
| **2 — Base userland** | bash, coreutils, git, tar | Command-line tools and an interactive environment, built against Layer 1 only. | Not started |
| **3 — Distribution** | pacman + repos | Package format, signed repository metadata, dependency solving, install/upgrade/rollback, profiles, mirrors. | Not started |
| **4 — Toolchain profiles** | MINGW64 / UCRT64 / CLANG64 | Named native targets and SDK bundles. | **Probably skip** — see below |

### The dependency rule

**Each upper layer depends downward. No lower layer depends upward. Ever.**

Package management never appears inside an adapter. A shell or coreutil never
calls Win32 or a Linux syscall directly.

This rule is **enforced mechanically** — see [Enforcement](#enforcement).
That is not ceremony. MSYS2 does not maintain its layering by discipline: its
layer 1 is a DLL, layer 2 binaries link against it, and the MINGW subsystems
exist *precisely because they do not link it*. The boundary is an ABI and the
linker refuses to let anyone cheat. **A Rust workspace has no equivalent
forcing function** — crates compile together, so a layer boundary is exactly
as real as the discipline maintaining it.

### Why Layer 4 is probably not needed

MINGW exists so you can build binaries that **avoid** the POSIX emulation
DLL. Rusty Mill has no emulation DLL to avoid — that is already an explicit
non-goal. Skipping Layer 4 is therefore not a bet on Rust tooling; it is a
direct consequence of a decision already taken. Recorded here as a deliberate
skip rather than an omission.

## Layer 1 in detail

### What is in it

| Crate | Role |
|---|---|
| `contract` | The portable API boundary. Traits and types only, no OS-specific behavior, no third-party OS adapters. **Depends on nothing in-workspace.** |
| `compat` | Native adapters implementing `contract` per host. |
| `conformance` | Cross-cutting verification. Not a layer — it sits beside Layer 1 and may inspect the adapters precisely because measuring them is its job. |

### The closed responsibility vocabulary

A correct dependency graph does not stop Layer 1 becoming a broad,
all-purpose runtime. That is the failure mode observed in `rustils`, whose
layering is clean but whose Layer-1 trait crate carries networking and
sandboxing — scope `msys-2.0.dll` never had. **Its architecture is not
tangled; it is simply wider than a Layer-1 boundary should be**, which is
what makes it hard to hold in your head.

So every public Layer-1 API root declares exactly one responsibility, drawn
from a closed set:

`paths` · `filesystem` · `process` · `terminal` · `environment` · `locking` ·
`standard-directories` · `capabilities` · `errors`

Declared as a doc-comment line on the item:

```rust
/// Filesystem operations scoped to a single root directory.
/// Layer-1 responsibility: filesystem
pub trait FsRoot { /* … */ }
```

Rules:

- A **type** may take the responsibility of the API it supports —
  `ProcessSpec` is `process` — rather than inventing a catch-all.
- `errors` covers the platform error model itself. It is **not** a licence to
  add arbitrary functionality under an error-shaped name.
- **If nothing in the vocabulary fits, the surface does not belong in Layer
  1.** That is the rule working, not the vocabulary being too small. Widening
  the list enlarges what Layer 1 is permitted to be, so it is an architecture
  change and belongs in a reviewed edit to this file.

The vocabulary is closed rather than free text for one reason: **free text
cannot be checked.** A prose instruction to "name the responsibility" holds
only while a reviewer remembers to notice it is missing, and noticing is the
step that gets skipped. A closed set turns omission into a build failure.

#### Two tags currently have no surface

`paths` and `environment` are in the vocabulary and classify nothing in
`contract` today. That is not slack in the vocabulary — it is two guarantees
in `CONTRACT.md` with no code behind them:

- The **Paths** row promises canonical, `/`-normalized-for-display paths.
  There is no path type and no normalization anywhere in the tree.
- The **Environment variables** row promises read/write access to the current
  process environment. There is no trait for it; `env` appears only as a
  field of `ProcessSpec`, which is child-process configuration and tagged
  `process`.

Both are open. Path translation in particular (`/c/Users` ↔ `C:\Users`) is
MSYS2's single most user-visible feature and has **no design here** — it stays
invisible only while `FsRoot` takes scoped relative paths, and surfaces the
moment anything accepts a user-typed or config-file path.

### Layer 0a: when an adapter may link a host binding

Default posture: **use standard and portable facilities first.** `compat`
satisfies every responsibility today through portable crates (`cap-std`,
`portable-pty`, `dirs`) and links no target-specific binding at all.

A Layer-1 adapter **may** depend on a Layer-0a binding, but only when *all*
of the following hold:

1. it is **target-gated** to its host OS;
2. it is **private to the adapter** and never appears in `contract`'s public
   API;
3. it is tied to **one** closed Layer-1 responsibility and a **demonstrated
   gap** in the current portable stack;
4. it is **explicitly allow-listed** as a `(crate, target, responsibility)`
   entry and checked by the gate.

This makes `rusty_win32` and `rusty_libc` legitimate *possible* inputs
without promoting either into a required foundation or a Rusty Mill product
layer.

## Where the Rusty Mill repos sit

| Repo | Layer | Relationship |
|---|---|---|
| `rusty_test` | 1 | This repo. The runtime contract, adapters, and conformance evidence. |
| `rusty_win32` | 0a | Windows-only safe Win32 bindings. **Not** a dependency of `rustils` — it is an *extraction donor*, code and design lifted by hand. |
| `rusty_libc` | 0a | Linux-only raw syscall bindings. A real optional dependency of `rustils`' Linux backend behind the off-by-default `track-p` feature. |
| `rustils` | 1 + 2 | Closest prior art: a clean, strictly-downward dependency graph, but a Layer-1 scope wider than ours. **Treat as a donor to raid for patterns, not a base to converge into.** Its `coreutils` composition root — target-gated backend selection behind a trait crate — is the pattern to copy when Layer 2 is built. |

Note the distinction between those two donors, because the maintenance
stories are opposite: **a dependency tracks upstream; an extraction forks it
and drifts.**

## Enforcement

Three mechanical gates, all in `crates/conformance/tests/`. Each exists
because the claim it checks had already rotted, or would have.

| Gate | Checks | Cannot check |
|---|---|---|
| `layering.rs` | Who may depend on whom. Every member has an explicit layer; `contract` has zero workspace deps; nothing depends upward or sideways. | Whether a surface *belongs* in its layer |
| `responsibility.rs` | What may live in Layer 1. Every public API root declares a responsibility from the closed set; no unknown tags; no unlisted Layer-0a binding. | Whether a declared tag is *honest* |
| `conformance` probes | That documented behavior matches measured behavior on every host, and fails the build on drift. | Behavior on hosts CI does not run |

Every gate in this repo follows one rule: **it must fail loudly on input it
cannot parse.** A checker that skips what it does not understand reports
success while policing nothing, which is worse than no checker. Each has a
test pinning that behavior.

### The question that stays human

**Is the declared responsibility honest?**

Nothing stops someone tagging a networking trait `filesystem` and the gates
waving it through. That is deliberate: mechanism handles the common failure
(nobody classified anything) so that review attention is spent only on the
rare one (someone classified it wrongly).

**Reviewers: for every new Layer-1 surface, ask whether the declared
responsibility is one `msys-2.0.dll` actually had.** If the honest answer is
"none of them," the surface belongs in another layer or outside the platform
— regardless of whether it is technically easy to add here.

## Open decisions

These are recorded rather than resolved. None blocks current work.

1. **Layer-1 crate naming.** The crates are still `contract` / `compat` /
   `conformance` rather than a Rusty Mill namespace.
2. **The first Layer-2 consumer.** One real developer CLI — not a demo —
   exercising scoped files, child processes, PTY interaction, and per-host
   state. It will expose which Layer-1 surface is genuinely needed next.
3. **Path translation.** See above. The first surface that accepts a
   user-typed path forces this.
4. **Whether `rusty_test` feeds the `rustils`/`rush` stack** or stays a
   separate, smaller SDK. This decides whether convergence with
   `rustils::platform` is a "watch it" or a "merge toward it" call.

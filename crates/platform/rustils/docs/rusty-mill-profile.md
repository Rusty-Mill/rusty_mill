# Rusty Mill repository standards profile — rustils

**Status:** Accepted — scoped to the filesystem domain only (see Accepted decision, below)
**Profile identity:** `rustils-profile` v0.2.0
**Path note:** `docs/05-governance/software-development/repository-profile.md` (Rusty-Mill AKB) says this profile lives "at a conventional reviewed path chosen by ecosystem RFC" — no such RFC exists yet. This path is a placeholder pending that RFC; treat it as provisional, not the final convention.

This document is rustils' half of the two-gate onboarding to the
[Rusty-Mill](https://github.com/Rusty-Mill/rusty_foundation_akb) ecosystem
(`RM-DEV-PROFILE-0005`: a repository without a current valid profile
cannot host an authorized implementation trial or publish a Rusty Mill
conformance/release claim). **This profile alone does not authorize a
trial** (`RM-DEV-GOV-0002`) — it satisfies the Repository entry gate's
own precondition (a current, valid, Accepted profile), not the trial's
Ownership/Bounds/Cross-cutting/Verification gates, which are tracked
separately in TRIAL-0002 itself. Outside the filesystem domain, rustils
makes no Rusty-Mill conformance or maturity claim; it continues operating
under its own `docs/rfc-v2.md` governance exactly as before.

**Companion gate status:** [TRIAL-0002](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/05-governance/implementation-trials/rustils-trial-proposal.md)
(revision 2) cites this repository as candidate qualified input evidence
for the **filesystem** domain, which holds an accepted Experimental
promotion decision as of 2026-08-10 (see [promotion review](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/02-capabilities/filesystem/promotion-review.md)).
Per TRIAL-0002's own gate table: Subject — Pass; Bounds — Pass; Ownership —
Qualified (named, not independent); Repository — was `Qualified` pending
this profile reaching Accepted status, which this revision resolves;
Verification — Qualified; Cross-cutting — Unknown; Learning value —
Qualified. The trial remains **Not authorized** until the remaining gates
clear — this profile update does not itself authorize it.
Process, networking, and security remain outside both this profile's
Accepted scope and TRIAL-0002's subject; they have no promotion review
(process, networking) or no accepted one (security has none written
either) yet.

## Profile identity / version

| Field | Value |
|---|---|
| Profile identity | `rustils-profile` |
| Profile version | 0.2.0 (Accepted, filesystem-scoped — see Accepted decision, below) |
| Compatibility generation | 0.2.0 binds a compatibility promise for the filesystem domain's Rusty-Mill-facing claims only (the R/D-level table in the filesystem promotion review, and this profile's own toolchain/dependency/CI bindings). It makes no promise for process/networking/security or for rustils' own public Rust API, which stays governed by `docs/versioning.md` as always. |

## Repository / components

| Field | Value |
|---|---|
| Repository | `baileyrd/rustils` |
| Governed crates | `platform`, `platform-linux`, `platform-windows`, `platform-bsd`, `platform-mock`, `platform-parity`, `winargv`, `coreutils` (workspace, `Cargo.toml`) |
| Repository class (per AKB `docs/04-ecosystem/repository-strategy.md` taxonomy) | Proposed: **Core platform** (capability framework + common APIs in one workspace) — not yet assigned; per that document, "final crate and repository names are assigned through an RFC after capability boundaries exist." rustils predates that RFC process, so this is a self-assessment, not an accepted classification. |
| Ownership | Single maintainer today (`baileyrd`); see AKB `governance.md` roles — one person may hold several roles initially |

## Architecture / domain inputs

| Field | Value |
|---|---|
| AKB architecture-model generation targeted | v1.99.0, commit `85c5c0d1b6bed541c13c0caa9f5f5f55ca442e25` (no AKB release tags exist yet — pinned by commit) |
| Capability domain bound by this profile generation | `rm.filesystem.*` ← `platform::fs` (`Dir`/`File`) — Experimental, per the filesystem [promotion review](https://github.com/Rusty-Mill/rusty_foundation_akb/blob/main/docs/02-capabilities/filesystem/promotion-review.md) accepted 2026-08-10, scoped exactly to that review's R/D-level table (Linux `open_dir`/`create_dir` R2, `write_atomic` D2; Windows same ops R2 link-confinement-only, `write_atomic` D1; everything else R1 on both; no R3/D3 anywhere) |
| Capability domains proposed but **not bound** by this generation | `rm.process.*` ← `platform::process`; `rm.networking.*` ← `platform::net`; `rm.security.random` ← `platform::security::Csprng`; `rm.security.restricted-execution` ← `platform::security::Sandbox`; `rm.security.secret` ← `platform::security::CredentialStore` (gated, no live forcing consumer). Each remains `Draft domain analysis` in the AKB with no accepted (process/networking: no written) promotion review. Binding any of these requires its own profile revision once its domain accepts Experimental promotion — not inferred from this one. |
| Known taxonomy gap | rustils' `platform::events::SignalSource` (deferred-signal delivery) has no obvious home in the AKB's current capability taxonomy (`docs/02-capabilities/`) — it isn't `rm.ipc.byte-pipe`, and there's no visible signals/events domain folder. Not part of this profile's bound scope; open question for whoever eventually proposes mapping it. |
| Non-goals inherited from rustils' own `rfc-v2.md` that predate Rusty-Mill | Terminal, PTY, Tun, Windowing, Registry/Config remain gated on their own named forcing consumers per `docs/architecture.md` — not proposed for Rusty-Mill domain mapping in this pass |

## Trial / maturity

| Field | Value |
|---|---|
| Approved trial class | None yet. This profile reaching Accepted status resolves TRIAL-0002's Repository gate precondition (`RM-DEV-PROFILE-0005`); it does not itself authorize the trial — Cross-cutting (`Unknown`) and Ownership (`Qualified`, independence-waived) remain separately unmet per `RM-TRIAL-ENTRY-0002`'s conjunctive rule. |
| Nonclaims | This profile makes no claim that any rustils capability outside filesystem is Rusty-Mill Draft/Experimental/Stable. For filesystem itself, this profile's Accepted status is a repository-side precondition only — it does not itself grant Experimental maturity (the AKB's own promotion-review path already did that, independently) and does not authorize implementation, per `RM-DEV-GOV-0002`/`RM-FILESYSTEM-PROMOTION-0001`. |

## Toolchain

| Field | Value |
|---|---|
| Edition | 2021 (`Cargo.toml`) |
| MSRV | 1.75, stable channel; CI matrix runs `stable` and `1.75` (`.github/workflows/ci.yml`) |
| Opt-in higher-MSRV tracks | `track-p` (platform-linux, `rusty_libc` raw-syscall backend, MSRV 1.88) and `track-w` (platform-windows, `rusty_win32`, MSRV 1.88) — feature-gated so the workspace floor stays 1.75 |
| Targets | Linux (glibc), Windows (MSVC), BSD family (macOS/FreeBSD/OpenBSD/NetBSD/DragonFly) via `platform-bsd`, net-only |
| SDKs/linkers | None beyond the standard Rust toolchain + platform C library; no vendored SDKs |

## Rules

| Field | Value |
|---|---|
| Inherited standards version | Rusty-Mill AKB `docs/05-governance/software-development/` at commit `85c5c0d1b6bed541c13c0caa9f5f5f55ca442e25` — this generation is now this repository's bound standards baseline (`RM-DEV-PROFILE-0001`), scoped to the filesystem domain per Architecture/domain inputs, above |
| Strengthened local rules | `#![forbid(unsafe_code)]` in `platform` (portable trait crate has zero unsafe, stricter than a workspace-wide unsafe budget would require); `#![deny(unsafe_code)]` re-opened narrowly inside each backend's `sys/` module only (`platform-linux`, `platform-windows`, `platform-bsd` `lib.rs`) |
| Declared non-applicability | Accessibility/i18n cross-cutting standards (`secure-inclusive.md`) — rustils is a Layer 2 OS-abstraction library with no user-facing surface; reviewed non-applicability, not a silent skip, per `RM-DEV-PROFILE-0002` |

## Unsafe / FFI

| Field | Value |
|---|---|
| Crates/modules carrying unsafe | `platform-linux/src/{ffi,sys}/`, `platform-windows/src/{ffi,sys,util}/`, `platform-bsd/src/{ffi,sys}/` only — every other module denies `unsafe_code` |
| Budgets | Not yet formally counted/tracked against an AKB unsafe budget — gap, tracked as a follow-up once the companion domain-authorization work defines what "budget" means for this repo |
| Owners | Single maintainer (`baileyrd`) |
| Invariants/audits | Documented per-block per rustils' own convention ("each block with a documented invariant" — see each backend crate's `lib.rs` module doc); no independent AKB-side unsafe audit has occurred |

## Dependencies

| Field | Value |
|---|---|
| Policy | `cargo-deny` (`deny.toml`): licenses allow-listed to MIT/Apache-2.0/Unicode-DFS-2016/Unicode-3.0; yanked crates denied; unknown git sources denied |
| Lock/vendor strategy | `Cargo.lock` committed (standard cargo resolution); no vendoring |
| Rev-pinned git dependencies (explicit allow-list) | `rusty_libc`, `rusty_win32`, `rusty_regx`, `rpath` — each pinned by rev, bumped deliberately, never tracking a branch |
| Banned | `tokio`, `async-std` — no async runtime may enter the tree (rustils' own architectural rule, RFC v2 §5, also documented in `docs/architecture.md`'s Execution and concurrency model section as the reason async support is an external-reactor escape hatch rather than an in-tree runtime dependency) |
| Advisories | `yanked = "deny"`; no `cargo audit`/RustSec advisory-database check currently wired into CI — gap |
| Update cadence | Ad hoc, maintainer-driven; no scheduled cadence defined yet |

## Verification

| Field | Value |
|---|---|
| Required assertions/cases | Behavior specs (`docs/behavior/*.md`) written before parity tests, per rustils' own testing discipline (`rfc-v2.md` §9) |
| Conformance mechanism | Parity suite: identical test source run on Linux + Windows CI legs, asserting identical observable behavior; `platform-bsd` verified on real macOS/FreeBSD hardware in a separate CI job |
| Mock-first coverage | `platform-mock` carries the bulk of consumer-logic unit coverage |
| Fuzz/model tests | `winargv` fuzzed against an argv-echo oracle (`fuzz/`, `.github/workflows/fuzz.yml`) |
| Platforms/providers exercised in CI | `ubuntu-latest` + `windows-latest`, both `stable` and MSRV toolchains; `track-p`/`track-w` legs on stable only; privileged `tun` test job; separate real-hardware BSD job |
| Miri | Not currently run in CI — gap versus rustils' own `rfc-v2.md` §9 commitment ("Miri: on everything it can execute") |

## Performance

| Field | Value |
|---|---|
| Scenarios/baselines/budgets | None formally defined yet — no benchmark suite exists in this workspace today. Gap; not addressed in this pass. |

## Cross-cutting

| Field | Value |
|---|---|
| Security | CSPRNG (`Csprng`), Landlock/seccomp sandboxing (`Sandbox`), OS trust-anchor loading (`TrustAnchors`) — see `docs/behavior/security.md` |
| Privacy | Not separately modeled; out of scope for a Layer 2 OS-abstraction library |
| Accessibility / internationalization | Declared non-applicable (see Rules, above) |
| Observability | Not formally instrumented; errors carry OS-code + kind context (`platform::error`, two-axis error model, `rfc-v2.md` §5.5) but no tracing/metrics story exists |
| Operations | No release automation beyond `CHANGELOG.md` + manual version bumps per `docs/versioning.md` |

## CI / release

| Field | Value |
|---|---|
| Pinned gates | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, plus the `track-p`/`track-w`/privileged-`tun`/BSD-hardware legs described above (`.github/workflows/ci.yml`) |
| Runner trust | GitHub-hosted runners (`ubuntu-latest`, `windows-latest`) + a separate real-hardware BSD runner; no self-hosted/attested build environment |
| Artifacts / provenance | None published — `publish = false` in `Cargo.toml`; not on crates.io |
| Publication authority | N/A — no publication has occurred |
| Merge strategy | Merge commit (matches AKB's own `CONTRIBUTING.md` convention, `ATLAS-TOOL-0040`) |

## Exceptions

Formalized as scoped, owned, expiring exceptions per the AKB's own governing conclusion ("exceptions are scoped, owned, expiring, and visible; suppression without rationale is not an exception process") rather than left as informal prose gaps, now that this profile is Accepted.

| ID | Scope | Owner | Rationale | Revisit trigger |
|---|---|---|---|---|
| `EXC-RUSTILS-0001` | No unsafe-code budget tracked against an AKB-style count | baileyrd | Existing unsafe scoping (`sys`/`ffi` modules only, `#![forbid(unsafe_code)]` elsewhere) is real and enforced by the compiler, but no numeric budget or audit trail exists yet | Before any Stable promotion claim for filesystem, or any domain whose unsafe surface materially grows |
| `EXC-RUSTILS-0002` | No `cargo audit`/RustSec advisory-database scanning in CI | baileyrd | `deny.toml` denies yanked crates but does not check the RustSec advisory database | Before any Stable promotion claim, or on request from a security reviewer once one is named (see TRIAL-0002's own Ownership waiver) |
| `EXC-RUSTILS-0003` | Miri not wired into CI | baileyrd | rustils' own `rfc-v2.md` §9 already commits to this; it just isn't automated yet — manual `cargo miri test` runs are ad hoc | Before any Stable promotion claim for a domain with meaningful unsafe surface (filesystem's is `sys`/`ffi`-scoped and Miri-clean today by convention, not yet by CI gate) |
| `EXC-RUSTILS-0004` | No performance-baseline suite | baileyrd | No benchmark scenarios exist in this workspace; filesystem's promotion review already excludes any native-performance claim for this reason | Before any native-performance claim for filesystem or any other domain |

None of these exceptions weaken a foundation safety, authority, evidence, or release rule (`RM-DEV-PROFILE-0002`) — they disclose verification and tooling gaps against an Experimental-level domain, not a Stable one, and Stable promotion is explicitly gated on closing them.

## Accepted decision

**Decision date:** 2026-08-10. **Decided by:** baileyrd — the same bootstrap staffing basis as filesystem's own `FS-EXP-W001` and TRIAL-0002's `TRIAL-0002-W002`, disclosed rather than assumed independent.

**Exact scope accepted:** every field table above (Repository/components, Architecture/domain inputs, Trial/maturity, Toolchain, Rules, Unsafe/FFI, Dependencies, Verification, Performance, Cross-cutting, CI/release, Exceptions), bound to this repository's state at commit `cc1c699130c1ed92428e2a9003f81dc0732e0305` and scoped to the **filesystem domain only**, per the narrowed Architecture/domain inputs table above. This is a real, if narrow, generation — not a placeholder.

**Waiver `RUSTILS-PROFILE-W001` (independent review):** granted, same basis as every other bootstrap waiver in this thread (`FS-EXP-W001`, `TRIAL-0002-W002`). This profile's Accepted status is solo-reviewed, not independently reviewed. Revisit trigger: before any Stable promotion claim for filesystem, or when a second accountable person is available for either repository, whichever comes first.

**What this decision does not do:** it does not authorize TRIAL-0002 (Cross-cutting and Ownership-independence remain separately unmet there); it does not bind process, networking, or security (explicitly excluded, above); it does not close any of the four formalized exceptions; and per `RM-DEV-GOV-0002` it does not itself select an API, crate boundary, or provider choice — rustils' own trait/backend shapes remain governed by `docs/rfc-v2.md` exactly as before.

---

## Open items before this profile's scope can widen

1. Process, networking, and security each reaching an accepted Experimental promotion decision in the AKB, then a follow-on profile revision binding them — none are bound by v0.2.0.
2. Resolve the `SignalSource` taxonomy gap (no clear `rm.*` home) — flagged in TRIAL-0002, unresolved.
3. Close `EXC-RUSTILS-0001`–`0004` (unsafe budget, advisory scanning, Miri-in-CI, performance baselines) — tracked as formal exceptions above, not yet closed.
4. A `RM-<DOMAIN>-<CAPABILITY>-NNNN`-style requirement-identifier mapping for rustils' existing behavior-spec assertions, once domain identities are accepted (today rustils' own `docs/behavior/*.md` files are the normative source; they don't yet cite AKB requirement IDs because none exist for these domains yet).

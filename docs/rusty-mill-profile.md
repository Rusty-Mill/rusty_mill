# Rusty Mill repository standards profile — rustils

**Status:** Draft
**Profile identity:** `rustils-profile` v0.1.0
**Path note:** `docs/05-governance/software-development/repository-profile.md` (Rusty-Mill AKB) says this profile lives "at a conventional reviewed path chosen by ecosystem RFC" — no such RFC exists yet. This path is a placeholder pending that RFC; treat it as provisional, not the final convention.

This document is rustils' half of the two-gate onboarding to the
[Rusty-Mill](https://github.com/Rusty-Mill/rusty_foundation_akb) ecosystem
(`RM-DEV-PROFILE-0005`: a repository without a current valid profile
cannot host an authorized implementation trial or publish a Rusty Mill
conformance/release claim). **This profile alone does not authorize
anything** (`RM-DEV-GOV-0002`) — the companion gate is an AKB-side
ADR/RFC authorizing rustils' capability domains for Experimental work
(`RM-DEV-GOV-0001`), which is a **separate, not-yet-started** piece of
work in the Rusty-Mill repo itself. Until that lands, rustils makes no
Rusty-Mill conformance or maturity claim; it continues operating under
its own `docs/rfc-v2.md` governance exactly as before.

## Profile identity / version

| Field | Value |
|---|---|
| Profile identity | `rustils-profile` |
| Profile version | 0.1.0 (Draft — first publication) |
| Compatibility generation | N/A until Accepted; a Draft profile makes no compatibility promise |

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
| Capability domains proposed for mapping (not yet authorized — see status note above) | `rm.filesystem.*` ← `platform::fs` (`Dir`/`File`); `rm.process.*` ← `platform::process` (`Spawner`/`Child`); `rm.networking.*` ← `platform::net` (`Net`, TCP/Unix/UDP); `rm.security.random` ← `platform::security::Csprng`; `rm.security.restricted-execution` ← `platform::security::Sandbox` (Landlock/seccomp); `rm.security.secret` ← `platform::security::CredentialStore` (gated — no live forcing consumer, see `docs/architecture.md`'s gated-surfaces table) |
| Known taxonomy gap | rustils' `platform::events::SignalSource` (deferred-signal delivery) has no obvious home in the AKB's current capability taxonomy (`docs/02-capabilities/`) — it isn't `rm.ipc.byte-pipe`, and there's no visible signals/events domain folder. Flagging as an open question for the companion AKB-side RFC rather than guessing a mapping here. |
| Non-goals inherited from rustils' own `rfc-v2.md` that predate Rusty-Mill | Terminal, PTY, Tun, Windowing, Registry/Config remain gated on their own named forcing consumers per `docs/architecture.md` — not proposed for Rusty-Mill domain mapping in this pass |

## Trial / maturity

| Field | Value |
|---|---|
| Approved trial class | None yet — `RM-DEV-GOV-0001` blocks any implementation trial from starting before its domain is AKB-authorized. rustils' existing code is **not** a Rusty-Mill implementation trial; it's pre-existing, independently-governed work being proposed for domain mapping. |
| Nonclaims | This profile makes no claim that any rustils capability is Rusty-Mill Draft/Experimental/Stable (per AKB ADR-0154's conjunctive maturity gates) until the companion domain-authorization ADR/RFC is Accepted and this profile itself reaches Accepted status. |

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
| Inherited standards version | Rusty-Mill AKB `docs/05-governance/software-development/` at commit `85c5c0d1b6bed541c13c0caa9f5f5f55ca442e25` (Draft binding — not yet reviewed/accepted as this repo's binding generation) |
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

| Field | Value |
|---|---|
| Active exceptions | None filed. Every "gap" noted above (unsafe budget, advisory scanning, Miri-in-CI, performance baselines) is disclosed as a known gap rather than filed as a formal, expiring, owned exception — filing those properly is follow-up work once this profile and the companion domain-authorization RFC are further along. |

---

## Open items before this profile can leave Draft

1. Companion AKB-side ADR/RFC authorizing rustils' proposed capability domains (`RM-DEV-GOV-0001`) — not started; separate work in the `Rusty-Mill/rusty_foundation_akb` repository.
2. Resolve the `SignalSource` taxonomy gap (no clear `rm.*` home) with that RFC.
3. Formal unsafe-code budget, RustSec advisory scanning in CI, Miri in CI, and a performance-baseline suite — disclosed gaps above, not yet closed.
4. A `RM-<DOMAIN>-<CAPABILITY>-NNNN`-style requirement-identifier mapping for rustils' existing behavior-spec assertions, once domain identities are accepted (today rustils' own `docs/behavior/*.md` files are the normative source; they don't yet cite AKB requirement IDs because none exist for these domains yet).

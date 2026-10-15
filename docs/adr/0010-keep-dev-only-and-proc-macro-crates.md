# ADR-0010: Keep dev-only and proc-macro crates external

Status: Accepted
Date: 2026-10-15

## Context

ADR-0002 exempts dev-only external dependencies from the sovereignty tiers
when they do not ship, and excludes proc-macro workspace members from the
meaning of "external". It does not say whether specific crates will stay
external or be replaced. Issue #707 asks for that decision, so the
"Sovereign foundation" goal in `ARCHITECTURE.md` has a recorded answer
rather than an open question.

Seven crates are in scope. Counts are direct dependencies in workspace
`Cargo.toml` files under `crates/` plus the root `Cargo.toml`, frozen Nexus
crates excluded from the verdict, checked 2026-10-15. This is not a vetted
audit.

| Crate | Where it is used | Ships in a release artifact |
| --- | --- | --- |
| `criterion` | `[dev-dependencies]` of `rush`, `rusty_multimodal_db`, `rusty_acp` (plus Nexus crates) | no, benchmarks only |
| `proptest` | `[dev-dependencies]` of `rusty_bbp` | no |
| `tokio-test` | `[dev-dependencies]` of the `rusty_adk` crates | no |
| `libfuzzer-sys` | `fuzz/` crates of `rush`, `rusty_croc`, `rusty_regx`, `rusty_tls`, `rusty_lsp`, `rusty_term`, `rustils` | no, fuzz targets only |
| `syn`, `quote`, `proc-macro2` | `[dependencies]` of four proc-macro crates: `rusty_err/derive`, `rusty_tokio-macros`, `adk-macros`, `rusty-db-derive` | no runtime code; they run at compile time only |

An in-house replacement exists for none of the seven. One partial precedent
does: `rusty_serde_derive` is written directly on `proc_macro` with no
`syn` or `quote`, and `rusty_json-derive` has no `syn`/`quote` dependency
either. Both derive over a small, fixed input shape.

## Decision

Keep all seven external. No replacement work is scheduled.

| Crate | Verdict | Reason |
| --- | --- | --- |
| `criterion` | Keep | Dev-only. Statistical benchmarking is a large body of work with no effect on shipped code. |
| `proptest` | Keep | Dev-only. It is useful as an independent oracle, which is the role ADR-0002 gives dev-only dependencies; a self-written generator would share the author's blind spots. |
| `tokio-test` | Keep | Dev-only, and it tests code written against `tokio`. It would go if the `rusty_adk` crates stopped using `tokio`. |
| `libfuzzer-sys` | Keep | Fuzz crates are separate, never published, and bind to LLVM's libFuzzer, which has no Rust-only equivalent to write. |
| `syn`, `quote`, `proc-macro2` | Keep for the four crates named above | They parse arbitrary Rust items (generics, attributes, where clauses) at compile time. A replacement means a Rust parser, which is large and security-sensitive to get wrong, and it buys no sovereignty at run time because nothing from these crates is in the compiled artifact. |

None of this changes a crate's tier. Under ADR-0002 the dev-only and fuzz
dependencies are exempt, and the proc-macro crates keep whatever tier their
`[dependencies]` give them. A proc-macro crate that needs `syn` is not
Sovereign for that reason alone.

New proc-macro crates should first try the `rusty_serde_derive` approach
(`proc_macro` only) when the input shape is small and fixed, and use `syn`
only when they must parse arbitrary items.

## Alternatives considered

**Replace `syn`/`quote`/`proc-macro2` with a first-party parser.** Rejected for
now. It is the only item here with real cost, and its benefit is limited to
the build graph: the crates do not ship. The four crates that use `syn`
would each need rewriting and re-verifying.

**Replace the dev-only crates.** Rejected. They have no effect on shipped
code, and writing a benchmark harness, a property tester and a fuzz driver
would use time that the runtime-dependency work in ADR-0002 needs more.

## Revisit when

Reopen this decision, per crate, if any of these happens:

- A `syn`-based proc-macro crate becomes a Sovereign-tier requirement, or a
  release product needs a build with no registry crates at all.
- A security advisory or maintenance halt hits `syn`, `quote`,
  `proc-macro2`, `criterion`, `proptest` or `libfuzzer-sys` that is not fixed
  within one release cycle.
- The number of crates that need `syn` falls to one or none, or the
  remaining ones can all use the `proc_macro`-only approach, which makes
  dropping it cheap.
- A dev-only dependency starts to ship (moves to `[dependencies]` of a
  published crate, or is enabled by a default feature).
- The `rusty_adk` crates stop using `tokio`, which retires `tokio-test`.

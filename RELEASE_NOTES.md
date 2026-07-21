# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracks notable changes to this repo, one entry per merged PR against `main`,
newest first (no version tags yet — this is pre-1.0).

---

## PR TBD — Skeleton crate + seam rule (mission handoff step 1)
**2026-07-21** · (not yet pushed — link once merged)

- **Added:** `Cargo.toml` (zero runtime deps, `[lints.rust]` forbidding
  `unsafe_code`), `src/lib.rs` crate-level docs stating the mission/scope/
  seam rule, `.gitignore`, and a Rust CI workflow (`cargo fmt`/`clippy -D
  warnings`/`cargo test`) now that a manifest exists to run it against.
- **Changed:** README and ARCHITECTURE rewritten from the greenfield
  placeholder to the real mission (sans-IO HTTP/1.1 core + `Url`,
  replacing donor code in `rusty_request`/`rusty_tail`), with a boundary
  table naming the planned (not yet built) sans-IO core and sync/async
  transport adapters.
- **Known limitation, stated plainly:** this is the skeleton only — no
  parsing/serialization code has landed. `Url` and the sans-IO message
  core are next (step 2 of the handoff plan), followed by adapters and
  the migration PRs.
- **Correction to the source handoff, worth recording:** the handoff
  described all four `rusty_tail` donor sites as hand-rolled HTTP. Source
  review found `ts-cli/src/localapi.rs` and `ts-localapi/src/lib.rs`
  actually build on `hyper`'s HTTP/1 client/server connection machinery —
  only their request routing and query-param parsing are hand-rolled, not
  the HTTP framing itself. `ts-control/src/controlhttp.rs` and
  `ts-derp/src/client.rs` are genuinely hand-rolled as described. This
  doesn't change the mission, but it changes what migrating the LocalAPI
  sites means: trading a mature, widely-used dependency (`hyper`) for this
  new crate, not de-duplicating hand-rolled logic — worth weighing
  deliberately when step 4's migration order gets there, not assumed.

## PR TBD — Bootstrap repo governance scaffolding
**2026-07-21** · (not yet pushed — link once merged)

- **Added:** PR templates (feature/bug_fix/docs/chore), issue templates
  (bug_report/feature_request), CONTRIBUTING, CODE_OF_CONDUCT, SECURITY,
  CHANGELOG, RELEASE_NOTES (this file), ARCHITECTURE, and an ADR seed via the
  repo-config skill.
- **Changed:** expanded README with a one-line project description and a
  Status section noting this repo has no code yet.
- **Known limitation, stated plainly:** this repo is greenfield — no
  `Cargo.toml` exists yet, so CI workflows were intentionally skipped (an
  always-red workflow is worse than none), and the ARCHITECTURE boundary
  table is left empty since there's nothing real to document yet.

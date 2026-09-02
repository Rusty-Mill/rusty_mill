*Point-in-time working document (software-architect lens), 2026-05-27. Superseded once the canonical docs (ARCHITECTURE.md, dev/error-handling.md, dev/testing-strategy.md, dev/coding-standards.md, refined PRDs) absorb it — do not cite as spec.*

# Software Architect Review — Rusty Keys

## 1. Scope & lens
Code-level structure and engineering practice: the unified error taxonomy (per-crate enums, thiserror/anyhow boundary, the "errors returned not panicked" rule, how errors map to the BLOCKED/ERROR strings the model sees), testing strategy (tiers + how to test LLM-dependent code), coding standards (MSRV, lints, features, async), CI/CD, trait-object-vs-generics consistency, and the public API surface across crate boundaries. Read PRDs 00/01/02/03/05/06 in full, skimmed 04/07/08, README, BACKLOG. I cite file:section where the corpus already covers a concern; peer reviews `systems-architect.md` (storage/DAG/concurrency) and `ai-engineer.md` (prompts/recall/eval) are orthogonal and I do not re-litigate them.

## 2. Validated gaps

- **G1 — No unified error taxonomy; per-crate `Error` enums are named but never defined.** `KernelError` (01:79,103), `PolicyError` (02:30 — defined as a bare `{reason: String}` struct, *not* an enum), `ToolError` (used everywhere in 03 but never declared), and bare `Result<()>` (03:236, 04:119, 05:217 — anyhow-style, alias unstated) coexist with no convention. There is no statement of which crate owns which error, how they convert (`From`/`#[from]`), or where the thiserror→anyhow boundary sits. ADR-001 promises compile-time correctness; an undefined error model undercuts it. Needs a dedicated doc.

- **G2 — The "errors returned, not panicked" rule (ADR-007) is asserted but has no enforcement or scope.** ADR-007 and 02:9 state the process never panics on policy violation, but the rule is never generalised (what about `unwrap()`/`expect()`/`panic!`/array indexing/`.await?` on a poisoned mutex?), nor is it backed by a lint (`clippy::unwrap_used`, `clippy::panic`). 02:54 itself contains `args["path"].as_str().unwrap_or("")` — benign, but the codebase needs a stated policy + lint to keep the invariant true.

- **G3 — The error→tool-result-string mapping is the model's prompt surface and is specified only by scattered example.** `BLOCKED by policy: {reason}` (02:223), `ERROR: …` / `ERROR (exit {code})` / `TIMEOUT: …` (01:98-104, 03:74-76), `ERROR: unknown tool` (02:241), `ERROR: MCP call failed` (07:110). `ToolStatus` is *reverse-engineered* from these prefixes by string match (04:40-41) — a fragile contract: any tool returning a message starting with "ERROR" is misclassified, and a real error whose text doesn't start with the magic prefix is counted as `Ok`. 03:364 already flags this as a seam ("`ToolResultClassifier` … currently inferred from result string prefix"). This needs to be a *defined, centralised* mapping (one formatter, one parser, ideally a structured `ToolOutcome` carried alongside the string), because verification correctness (NoToolErrors, 05:40) and entropy/attribution depend on it.

- **G4 — No testing strategy doc at all; `docs/dev/` does not exist.** BACKLOG phases are tagged H1/H2/H3 but no phase lists tests as a deliverable, and there is no unit/integration/property/snapshot tiering. Most critically: **the harness's entire thesis is "verification you can trust," yet there is no story for testing LLM-dependent code** (kernel loop, CriteriaJudge, consolidation, compaction). Without a fake/mock `LanguageModel` and golden-episode replay, every test either hits a live provider (nondeterministic, costs tokens, flaky CI) or doesn't exist. The `ai-engineer.md` review proposes a golden-episode *eval plan* (its §5, `docs/dev/eval-plan.md`); that is product-level maturity measurement. This gap is the *engineering* substrate beneath it — the seam that makes any LLM-touching code unit-testable at all. They share the episode-package fixture format (05:257) but are different docs.

- **G5 — Trait-object vs generics (monomorphization) is decided ad hoc, not as a stated convention.** ADR-010 says "trait objects add a vtable; negligible compared to LLM latency" — a real decision, but applied inconsistently and never written as a rule. `Box<dyn ToolFn>` (03:40), `Box<dyn Check>` (05:60), `Box<dyn Policy>` (06:25, 02:206), `Box<dyn SecurityCheck>` (02:147), `Arc<dyn McpClient>` (07:100) are all dynamic; meanwhile `Stream`/`Store` (03:234,252) are traits whose *storage form* in `Memory`/`Session` is never shown (Box? Arc? generic param `<S: Store>`?). `Session` (06:21) holds `Kernel`, `Verifier`, etc. as concrete types but `policy: Box<dyn Policy>`. The convention should be stated once ("trait objects at all plugin seams; generics nowhere in v1 because the harness is not in a tight loop") so reviewers can enforce it — and the `Stream`/`Store` ownership form must be pinned.

- **G6 — `async` in traits is used pervasively with no stated mechanism.** `Stream`/`Store` (03), `Check`'s exception `CriteriaJudge` (05), `McpClient` (07), `ToolFn::call` (02:106, 07:106) all declare `async fn` in trait position. Native async-fn-in-trait (stable since Rust 1.75) does **not** produce `dyn`-compatible traits without `#[allow(async_fn_in_trait)]` caveats or the `trait-variant`/`async-trait` crates — yet these same traits are used as `Box<dyn …>`/`Arc<dyn …>` (G5). This is a concrete compile-level decision (async-trait macro vs `-> impl Future + Send` desugaring) that MSRV and the coding-standards doc must pin, because it changes every trait signature in the corpus.

- **G7 — No coding-standards baseline: MSRV, rustfmt, clippy lint set, naming, feature-flag conventions are all unstated.** Nothing declares the minimum supported Rust version (matters: async-fn-in-trait needs ≥1.75; `let-else` etc.), no `rustfmt.toml`/`clippy.toml`, no lint-level policy (`#![deny(warnings)]`? `clippy::pedantic`? the `unwrap_used` lint from G2?), no naming guide (the corpus is already consistent — `XxxError`, `XxxPolicy`, `XxxCheck`, `rk://` events, `RUSTYKEYS_*` env, `mcp__server__tool` — but it's undocumented convention, not a written rule).

- **G8 — No CI/CD pipeline is described anywhere.** No build matrix, no `cargo clippy`/`fmt --check`/`test`/`cargo-audit`/`cargo-deny` gate, no MSRV check job, no release/publish story, no coverage. For a project whose selling point is correctness and auditability, the absence of a CI definition (and of any `.github/workflows`) is a first-order engineering gap. The frontend (Tauri/SolidJS, PRD 08) adds a second toolchain (node/vite) that CI must also cover.

- **G9 — Feature-flag conventions exist conceptually but are never assembled as a Cargo feature plan.** ADR-010 names `duckdb` optional; web tools gate on a *runtime* env var (`RUSTYKEYS_ALLOW_WEB`, 03:100) not a compile feature; gateway (`axum`), MCP, and frontend are separable but no `[features]` table maps feature → crate → optional deps. `systems-architect.md` flags the *matrix assembly* as architecture work (its §3); the *Cargo-level* convention (default features, `--no-default-features` build, which heavy dep each flag gates: `duckdb-rs`, `reqwest`, `axum`, tauri) is the coding-standards slice and is uncovered.

## 3. Already-covered / pruned

- **Policy-block recovery semantics** — covered: `01 §Tool dispatch and policy`, ADR-007, `02 §Integration with the tool registry`. The *flow* (block → string → model recovers) is sound; only the string *contract* (G3) and sync/async signature are gaps.
- **MCP failure→string mapping** — covered: `07 §Error handling` table is the most complete error-handling table in the corpus. It still uses the magic-prefix string (G3) but the failure modes themselves are handled.
- **CriteriaJudge graceful degradation** — covered (mechanism): `05 §Graceful degradation`. Note: `ai-engineer.md` §2.7 correctly flags that *passing-on-parse-failure* is a false-positive hazard; that's a behavioral fix in their lane, not an error-taxonomy gap in mine.
- **`max_steps` non-termination handling** — covered: `01 §Error handling`, `05 NoToolErrors`/`CleanTermination`. The loop-exit→verifier-catches path is well-specified.
- **Crate boundary intent (the DAG)** — partially covered: `06 §Cargo workspace layout` states "kernel cannot import feed/compose; observe cannot import compose; app imports everything; DAG." `systems-architect.md` G2/G3 owns the DAG's *completeness* (crate count, unstated compose→observe/feed edges). I do **not** re-flag the DAG; I add only the *public-API-surface* dimension (N3) which is distinct.
- **Tracer `!Send`, no-lock ownership** — covered: `04 §Rust advantages`. Good, concrete, correct.
- **`spawn_blocking` for SQLite / tokio::join! post-turn** — covered: ADR-003, ADR-012, `05 §Concurrency`. Out of my lens; systems-architect owns the SQLite-contention angle (their G9).

## 4. New gaps (beyond the original focus list)

- **N1 — Tool registration mechanism conflicts with itself.** `ToolRegistry` holds `HashMap<String, Box<dyn ToolFn>>` (03:40), but the built-in tools are free `#[tool]`-annotated async fns (03:24, 03:70, etc.) and the kernel calls `registry.tools()` / `.with_tools(...)` (01:24,36). How an aisdk `#[tool]` fn becomes a `Box<dyn ToolFn>` entry in the registry (and how the macro's aisdk-native tool object coexists with the harness's own `ToolFn` trait used for dispatch+policy) is never shown. This is the load-bearing seam between aisdk and the harness and is the most likely place a real implementation diverges from the spec. The coding-standards/feed PRD must show one concrete registration adapter.

- **N2 — `ToolFn` trait is never defined.** It is used as `Box<dyn ToolFn>` (03:40) and `impl ToolFn for McpToolFn` with `async fn call(&self, args) -> String` (07:105-112), but no PRD declares the trait. Its return type is `String` (not `Result`), which is *itself* the design that forces the magic-prefix encoding (G3) — worth stating deliberately: dispatch returns an infallible already-stringified result, errors are values not `Err`s at this boundary.

- **N3 — Public vs internal API surface is entirely unmarked.** Every type in the corpus is shown `pub`. Across seven-plus crates, nothing says what each crate *exports* vs keeps internal. The brief itself notes pre-code API freezing may be premature — agreed — so the recommendation is *not* to freeze signatures but to state a **visibility policy** (e.g. "crates expose the minimal trait + constructor; concrete impls `pub(crate)` unless a downstream crate names them; `app` is the only crate with a binary"). Without even the policy, every type defaults to `pub` and the boundary erodes on contact.

- **N4 — `Result<T>` (no error param) vs `Result<T, SpecificError>` are mixed within single PRDs.** 03 uses bare `Result<()>`/`Result<String>` (236, 257, 280) — implying a crate-level `type Result<T> = std::result::Result<T, anyhow::Error>` alias — *and* `Result<String, ToolError>` (24, 70). 05 uses bare `Result<()>` (217, 311) and `Result<Self>` (217). The thiserror-at-library-edges / anyhow-at-application-core boundary (G1) must declare where the alias is legal: my read is anyhow is acceptable inside `app`/binaries and consolidation glue, but library crates (kernel/constrain/feed/observe/compose/mcp/config) should return concrete `thiserror` enums on their public API. That rule is unstated.

- **N5 — `PolicyError` is a struct, not an enum, so failure attribution loses structure.** 02:30 `PolicyError { reason: String }`. But `05 §Failure attribution` maps blocks to `(category="permission_block", layer="constrain/policy")` and the security log records a `checker` field (02:136). A free-text `reason` means the compose layer can't programmatically distinguish a path-traversal block from a mode-gate block without parsing prose. An enum (`PolicyError::OutsideWorkspace`, `::ModeForbidden`, `::SecurityCheck{checker}`, `::WebDisabled`) would let attribution and the security event derive structurally rather than by string. This is the concrete first instance of the G1 taxonomy.

## 5. Recommended edits

### Concrete shapes I recommend

**Error taxonomy (for `docs/dev/error-handling.md`).** One `thiserror` enum per library crate, named `<Crate>Error`; cross-crate composition via `#[from]`. anyhow only in `app` (and post-turn glue that is genuinely best-effort). Sketch:
```rust
// constrain/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum PolicyError {
    #[error("path {0} escapes workspace root")]
    OutsideWorkspace(PathBuf),
    #[error("mode {mode} forbids tool {tool}")]
    ModeForbidden { mode: &'static str, tool: String },
    #[error("blocked by {checker}: {pattern}")]
    SecurityCheck { checker: &'static str, pattern: String },
    #[error("web access disabled; set RUSTYKEYS_ALLOW_WEB=1")]
    WebDisabled,
    #[error("blocked by approval gate")]
    ApprovalDenied,
}
```
The "errors returned not panicked" rule (ADR-007) becomes enforceable: workspace lint `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]` in library crates, allow-listed in tests.

**Tool-result string contract (resolves G3/N2).** Define a single owned type and one formatter/parser pair, used by both dispatch (02:228) and the Tracer's `ToolStatus` inference (04:40), replacing scattered `format!`:
```rust
pub enum ToolOutcome { Ok(String), Error(String), Blocked(String), Timeout(String) }
impl ToolOutcome {
    pub fn to_model_string(&self) -> String { /* "BLOCKED by policy: …" etc. */ }
    pub fn status(&self) -> ToolStatus { /* total, no prefix-guessing */ }
}
```
`ToolFn::call` returns `ToolOutcome` (or `Result<String, ToolError>` mapped to it at the registry), so status is carried structurally, not re-parsed from prose. This is the 03:364 `ToolResultClassifier` seam, pulled forward.

**Testing strategy (for `docs/dev/testing-strategy.md`).** Four tiers + the LLM seam:
1. *Unit* — pure logic: `WorkspacePolicy` boundary (canonicalize/`../`), each `SecurityCheck`, `ToolStatus` inference, recall scoring math, entropy heuristics, M-HIR arithmetic. No async, no I/O.
2. *Integration* — a real `Session` over a **`FakeLanguageModel`**: a `LanguageModel` implementation that returns scripted turns (a `Vec<ScriptedTurn>` of "emit these tool calls, then this final text"). This is the keystone — define it as a first-class test fixture in `kernel` (behind a `test-support` feature or `#[cfg(test)]`+exported test crate) so every crate above can drive a deterministic episode. Lets you assert: policy blocks surface as BLOCKED strings; verifier marks UNVERIFIED on tool error; `tokio::join!` post-turn ordering; compaction triggers.
3. *Property* (`proptest`) — invariants: no input path escapes the workspace; JSONL round-trips (`serde` encode→decode is identity); `ToolOutcome::to_model_string`→`status()` round-trips the correct status for arbitrary payloads (directly guards G3).
4. *Snapshot* (`insta`) — stable rendered surfaces: `VerificationReport::render()` (05:101), recall block, startup banner (06:214), `/mhir` output, episode-package JSON. These are the human/model-facing strings; snapshot them so refactors can't silently change the prompt surface.
   *Golden-episode replay*: a fixtures dir of frozen episode-package JSON (05:257); the replay harness feeds the recorded tool results through the verifier/outcome-classifier and asserts the same `EpisodeOutcome`. Shares the fixture format with `ai-engineer.md`'s eval-plan but tests the *deterministic* compose/verify logic, not model quality. Record real episodes once, replay forever — that is how "verification you can trust" becomes itself tested.

**Coding standards (for `docs/dev/coding-standards.md`).** State: MSRV (recommend pinning ≥1.82, well above the 1.75 async-fn-in-trait floor, in a `rust-toolchain.toml`); async-trait mechanism decision (recommend native async-fn-in-trait + `trait-variant` for the `Send` bound where `dyn`-compat is needed — and confirm each `dyn`-used trait either avoids async or wraps it); rustfmt default + a short `clippy.toml`; the trait-object convention (G5: trait objects at plugin seams, no generics in v1, pin `Stream`/`Store` as `Arc<dyn …>`); visibility policy (N3); feature-flag table (G9).

### Edit table

| target file | change | priority | depends-on |
|---|---|---|---|
| docs/dev/error-handling.md (new) | Author the unified taxonomy: one `thiserror` enum per library crate, `#[from]` composition, anyhow-only-in-app boundary, the ADR-007 no-panic rule backed by `unwrap_used`/`panic` lints. Include the `PolicyError` enum sketch and convert it from a struct. | P0 | G1,G2,N4,N5 |
| docs/dev/error-handling.md + prd/02 + prd/01 | Define the **tool-result string contract**: one `ToolOutcome` type, single formatter/parser, replacing magic-prefix inference in `ToolStatus` (04:40). Pull the 03:364 `ToolResultClassifier` seam forward. | P0 | G3,N2 |
| docs/dev/testing-strategy.md (new) | Author the four-tier strategy + the **`FakeLanguageModel`** scripted-turn fixture as the keystone for testing all LLM-dependent code; golden-episode deterministic replay over the 05:257 package format. | P0 | G4 |
| docs/dev/coding-standards.md (new) | MSRV + `rust-toolchain.toml`; rustfmt/clippy lint set; async-trait mechanism decision; trait-object-vs-generics convention; naming; visibility policy; Cargo `[features]` table. | P0 | G5,G6,G7,G9,N3 |
| .github/workflows/ci.yml (new) + coding-standards.md | Define CI: build matrix (stable + MSRV), `fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `--no-default-features` + `--all-features` builds, `cargo-audit`/`cargo-deny`, frontend (node/vite) job. Release/publish story. | P1 | G8,G9 |
| prd/03-feed.md | Define `ToolFn` trait (N2) and show one concrete adapter from an aisdk `#[tool]` fn into `Box<dyn ToolFn>` in the registry (N1) — the aisdk↔harness seam. Pin `Stream`/`Store` storage form (`Arc<dyn …>`). | P1 | G5,N1,N2 |
| prd/02-constrain.md | Convert `PolicyError` struct → enum so attribution (05) and `SecurityEvent` (02:136) derive structurally, not by parsing `reason`. | P1 | N5,G1 |
| prd/00-overview.md (ADR) | Add an ADR (or extend ADR-010) stating the trait-object-everywhere-at-seams + async-trait mechanism decision, so it's a recorded choice not an inferred one. | P2 | G5,G6 |
| BACKLOG.md | Add a cross-cutting "engineering substrate" note: error-handling + testing-strategy + coding-standards + CI land *with Phase 1*, not after; each phase's "done" includes its tier of tests (fake-LLM integration tests arrive with Phase 1's Session). | P1 | G4,G8 |

## 6. Cross-persona dependencies
- **AI engineer:** we share the episode-package fixture format (05:257). Their `docs/dev/eval-plan.md` measures *model/maturity* (H1→H3 gates, judge-unavailable rate); my `docs/dev/testing-strategy.md` provides the *deterministic substrate* (FakeLanguageModel, replay of compose/verify logic). The split must be explicit so the two docs reference rather than duplicate. Their CriteriaJudge parse-failure fix (their §5) interacts with my error taxonomy — `judge_unavailable` should be a typed state, not a passing `CheckResult`.
- **Systems architect:** they own the crate DAG completeness and on-disk schemas; I own the public-API *visibility policy* layered on those crates (N3) and the Cargo *feature* plumbing beneath their feature *matrix* (G9). Their serde-rename pinning (their N4) and my `ToolOutcome`/error-enum serde encodings must agree (one convention: `rename_all="snake_case"`). The torn-line JSONL policy (their G7) is testable via my property-test tier — co-locate.
- **Security/safety persona:** the `PolicyError` enum (N5) and the `unwrap_used`/`panic` lint policy (G2) are partly a security surface; the `Bypass` mode lints and the security.jsonl structured fields depend on the same enum decision.
- **Product/roadmap persona:** sequencing the engineering-substrate workstream into Phase 1 (my BACKLOG edit) and the CI gate thresholds are their call to ratify.

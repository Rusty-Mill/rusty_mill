# Rusty Mill → Atlas Evidence Review

> **Historical record, not the current assessment.** This review is pinned
> to Rusty Mill `06ca8669` and treats PR #131 as open; #131 has since
> merged (`main`'s `cfea436`), and the workspace has grown to 183 members.
> It remains useful evidence for the ATLAS-300 feature-additivity trigger
> and the other findings below, but a reader checking current state should
> use `docs/adr/0002-dependency-sovereignty-policy.md` and the workspace's
> present commit, not this document's counts or its "currently open PR"
> framing.

Review date: 2026-09-02 (revision 2, corrected after source verification)
Rusty Mill evidence revision: [`06ca8669f38f80291a63308de7563bfea43caab5`](https://github.com/Rusty-Mill/rusty_mill/tree/06ca8669f38f80291a63308de7563bfea43caab5)
Atlas authority revision: [`390d6b0f217a7bdf425cc9ea82e81c27a649cd0c`](https://github.com/baileyrd/Atlas_Engineering_Standards_Library/tree/390d6b0f217a7bdf425cc9ea82e81c27a649cd0c)

Verification basis: every claim below was checked against a checkout of the Rusty Mill revision, the live GitHub pull-request and branch state on the review date, and a clone of the Atlas revision. All 39 Atlas requirement identifiers cited resolve to published headings at the Atlas revision.

## Executive conclusion

Rusty Mill is strong exercised evidence for Atlas's existing Cargo-workspace, monorepo, validation, provenance, and mechanism-versus-policy requirements. It is not currently fully Atlas-conformant.

The most important finding is that **ATLAS-300's deferred "Feature-flag architecture" trigger has fired**. Its published trigger reads: "Multiple supported feature combinations create real compatibility, dependency, or validation differences that require a shared policy." Rusty Mill produced two independent instances of that condition on the same day:

1. `rusty_request`'s optional `tokio` feature replaced its `rusty_tokio` backend at compile time. The full-workspace `--all-features` sweep that CI runs unified that feature on for every consumer in the graph, and the nine `rusty_tokio`-based crates proposed in PR #131 panicked at run time with "there is no reactor running". Package-scoped tests had passed. The defect was fixed at the source in PR #134 (merged as `9c33680`, an ancestor of the evidence revision) by making the feature genuinely additive: both backends compile in and each call picks the runtime actually driving its task.
2. Once #134 un-gated `rusty_request`'s HTTPS tests under `--all-features`, the same sweep compiled both `ring` and `aws-lc-rs` into rustls, and the test TLS server could no longer auto-select a crypto provider. PR #136 pinned the provider explicitly.

Both failures were caused by feature unification across the workspace graph, both were invisible to package-scoped validation, and both were resolved by honouring Cargo's existing rule that features must be additive. Atlas currently has no Cargo-feature requirements that name that rule or govern how workspace validation must exercise the unified graph.

Recommended disposition: **Redirect** the next Atlas documentation increment toward a bounded ATLAS-300 feature-architecture amendment centred on feature additivity, backed by a new immutable Rusty Mill evidence record. Separately, Rusty Mill should correct its branch protection and repository-map drift before claiming Atlas conformance.

## 1. Concise lessons learned

1. **Cargo features are graph-level behavior, not package-local switches.** A feature requested anywhere in the resolved graph changes the compiled behavior seen by every consumer of that package. A non-additive feature therefore breaks consumers that never asked for it.
2. **`--all-features` validates additivity; it is not a deployment profile.** A green result under `--all-features` proves the unified graph works together. A red result is a real defect in some package's feature design, as both #134 and #136 showed, and must not be dismissed as a stress artefact.
3. **Scoped validation must include transitive dependents under the same feature graph used by CI.** Rusty Mill's `affected_crates.py` computes reverse dependencies from `cargo metadata --all-features` precisely because CI builds with `--all-features`; its docstring states that the two graphs must match.
4. **A capability inventory does not establish runtime integration.** PR #131 reported 436/436 migration rows complete while the unified-workspace sweep exposed a runtime incompatibility. After that was fixed, the same PR now fails only on Windows, where two `rusty-meshed-cli` metrics tests time out on every retry. This directly validates ATLAS's non-transitive maturity model.
5. **A monorepo is a coordination boundary, not a claim of one product or one lifecycle.** Rusty Mill explicitly preserves independent crate APIs, histories, ADR series, and purposes while sharing a lockfile and CI.
6. **History preservation has architectural value.** `git subtree` retained pre-consolidation history needed for later defect and duplication analysis.
7. **Dependency deduplication must compare semantics, not names.** The repository inspector's clustering matched an HMAC module on path alone, and the first human verification then used a substring pattern (`fn hmac`) loose enough to accept `hmac_md5` and `hmac_sha1` as HMAC-SHA256. Two distinct failures, both recorded in the report so the claim is not re-proposed a third time.
8. **Mechanism can be shared while policy remains local.** The extracted retry engine unified backoff mechanics while preserving HTTP-idempotency and ACP-operation eligibility as separate policies.
9. **Platform evidence must say what actually executed.** `rusty_tokio`'s README states that its macOS/BSD integrations have never run on real hardware and that Windows has run once; its BSD paths are verified only by `cargo check` against Apple, FreeBSD, and NetBSD targets. The two real AF_UNIX defects (one Darwin, one OpenBSD) were caught by `rustils`' `platform-bsd` upstream CI before consolidation. The monorepo's own CI runs Linux and Windows only, so consolidation narrowed the executed platform set, and the repository says so rather than inheriting the upstream claim.
10. **Large monorepos need generated or mechanically checked maps.** Rusty Mill's hand-maintained README already contains stale and internally inconsistent relationship claims after rapid consolidation.

## 2. Formal Atlas crosswalk and gap audit

### Existing Atlas requirements supported by Rusty Mill

| Rusty Mill evidence | Atlas requirement(s) supported | Assessment |
|---|---|---|
| Explicit virtual workspace with 174 listed members, 9 explicit excludes, and `resolver = "2"` | `ATLAS-RWC-0001`, `0010`, `0020` | Strong exercised evidence |
| Shared `[workspace.package]` metadata and 167 `[workspace.dependencies]` entries, with documented local exceptions for incompatible versions, licenses, MSRVs, and release identities | `ATLAS-RWC-0030`, `0040`, `0080`, `0090`; `ATLAS-TOOL-0270` | Strong evidence for "shared policy without false uniformity" |
| First-party git dependencies converted to workspace path dependencies as crates migrate | `ATLAS-RWC-0050` | Strong exercised evidence |
| Root `Cargo.lock` and explicit lockfile effects in ADR-0001 | `ATLAS-RWC-0120`, `0130` | Strong exercised evidence |
| Root architecture document distinguishes the build/governance boundary from runtime and domain model, and gives a dependency-layering table | `ATLAS-TOOL-0230` | Conforms in substance; it treats repository and Cargo workspace as one boundary and does not discuss product identity |
| Root README, architecture document, per-crate documentation, and scoped ADR series | `ATLAS-TOOL-0240`, `0250`; `ATLAS-KNOW-0001` | Partially conforms; discoverable but not reliably current |
| Impact-aware CI selects changed crates plus transitive dependents; root manifest, lockfile, and `.github/` changes force a full sweep | `ATLAS-TOOL-0060`, `0070`, `0080`, `0280` | Strong implementation pattern |
| Linux and Windows jobs, explicit Windows-only exclusions (including a doctest fix in #133), a separate mingw cross-target check, and per-crate statements of which platforms actually executed | `ATLAS-TOOL-0180`, `0220`, `0280` | Generally aligned; evidence limits are documented |
| Required formatting and Clippy-with-denied-warnings jobs | `ATLAS-TOOL-0190`, `0200`, `0210` | CI behavior aligns, but enforcement is not protected |
| Consolidation ADR records remit, context, decision, alternatives, consequences, and authority split | `ATLAS-VAL-0010`, `0020`; `ATLAS-KNOW-0001`; `ATLAS-GOV-ADR-0001` | Strong evidence |
| Retry extraction preserves separate eligibility policy | `ATLAS-PHIL-0101` | Strong mechanism/policy example |
| PR #131 manifest completion followed by runtime-integration failure, with the unresolved `rusty_tokio` interop decision request as the open parent question | `ATLAS-LIFE-0020`, `0021`; `ATLAS-SPEC-0020` | Strong negative evidence validating existing maturity controls |

### Atlas gaps exposed by the evidence

| Gap | Evidence | Atlas disposition |
|---|---|---|
| No normative feature-flag architecture, and no statement that Cargo features must be additive | `rusty_request`'s non-additive `tokio` feature was unified on by the workspace `--all-features` sweep and panicked `rusty_tokio`-scheduled consumers on PR #131; fixed at source in #134 by restoring additivity | **Trigger fired:** activate the deferred ATLAS-300 topic |
| No rule that impact analysis and validation must use the same feature graph | Package-scoped tests and the full-workspace sweep exercised materially different graphs; `affected_crates.py` already enforces graph consistency locally as an undocumented convention | Add a validation-graph consistency requirement |
| No rule that a feature must not silently change runtime or environmental preconditions for all consumers | Before #134 the feature changed the required executor without any compile-time contract failure; #136 shows the same class for TLS provider selection in a third-party crate | Add feature-effect and runtime-precondition requirements, with additivity as the primary remedy |
| Monorepo map freshness is not mechanically protected | README says `rusty_simd` "is the one still outstanding" while the same file elsewhere says it is already merged and `Cargo.toml` lists it; README says `rusty_tokio` has no in-repo dependents while twelve workspace packages depend on it by `path` at the evidence revision, per `cargo metadata --all-features` (nineteen once PR #131 merged); `ARCHITECTURE.md` describes ATLAS-300 as a seed too draft to cite, which predates ATLAS-300's promotion | Existing `ATLAS-TOOL-0240` is directionally correct; add guidance/tooling evidence before adding a new MUST |
| Automated inspection output can overstate equivalence | Path-only clustering plus substring verification let the HMAC false positive survive two report versions | Capture as non-normative validation guidance; insufficient evidence for a new ecosystem-wide normative rule yet |
| Topology changes do not consistently crosswalk affected Atlas volumes | Consolidation PRs and ADR-0001 explain migration mechanics but do not identify continuing ATLAS-100/200/300/600 obligations | Existing `ATLAS-TOOL-0300` covers this; Rusty Mill needs adoption, not a new Atlas rule |

## 3. Draft Atlas amendment proposals

These are proposed requirement outcomes, not approved normative text or allocated identifiers.

### Proposal A — activate ATLAS-300 Feature-Flag Architecture

Amend the ATLAS-300 Purpose and Deferred sections to state that the feature-strategy trigger fired through Rusty Mill evidence (record it in the Atlas `docs-audit.md` finding table as a `DOC-nnn` row). Add a narrowly scoped chapter with requirements equivalent to:

1. **Features are additive.** Enabling a Cargo feature MUST NOT remove, replace, or change the behavior a consumer receives with that feature off. This restates Cargo's own reference rule as an Atlas requirement so that a violation is a defect, not a configuration choice. The preferred remedy for a feature that must select between backends is to compile both and dispatch at run time (the pattern PR #134 adopted), not a bridge and not a documented failure.
2. **Resolved feature sets are graph-level contracts.** A workspace must evaluate a package's effective features in the resolved dependency graph, not only the features declared by an individual consumer.
3. **`--all-features` validates additivity.** Required workspace validation SHOULD exercise the fully unified graph. A failure there is a defect in some package's feature design and MUST be root-caused, not waived. `--all-features` MUST NOT be represented as proof of any minimal or default profile, and required minimal/default profiles still need their own validation.
4. **Impact analysis and execution use the same graph.** Impact-aware CI MUST compute dependency reachability under the same feature assumptions as the validation commands it selects.
5. **Residual non-additive cases are explicit.** Where additivity is technically impossible (mutually exclusive TLS providers, allocators, wire or persistence formats), the package MUST document the supported profiles, their runtime or environmental preconditions, and MUST produce a clear compile-time or configuration error for an unsupported combination where feasible. Only when that is infeasible may a documented and tested runtime failure stand in.
6. **Feature effects must not be silent.** A feature that changes runtime, executor, I/O trait, allocator, TLS provider, serialization, wire, persistence, or other environmental preconditions must document that effect as part of the affected compatibility surface.

Anticipated objection: each Rusty Mill incident was a single package's defect, fixed locally without a shared policy. The response is that two independent incidents in one day, in first-party and third-party packages, both surfaced only through workspace-level unification, and the fix that prevents recurrence (graph-consistent validation plus an additivity rule) is workspace policy, not a package patch.

### Proposal B — add Rusty Mill to evidence provenance

Add an immutable evidence record to `docs/reference/evidence-provenance.md` in the ledger's existing shape (proposed identifier `EVID-RUSTYMILL-2026-09-02`), with the four-field header table, blob links pinned to revision `06ca8669`, and a limitations paragraph. Source artifacts at that revision:

- root `Cargo.toml` and `Cargo.lock`;
- `ARCHITECTURE.md` and workspace ADR-0001;
- `.github/workflows/ci.yml` and `.github/scripts/affected_crates.py`;
- `repo-inspector-report.md`;
- `crates/rusty_request/Cargo.toml` (the additive-feature rationale comment) and `crates/rusty_request/src/rt.rs`;
- `crates/rusty_tokio/README.md` (executed-versus-checked platform statement) and `crates/rusty_tokio/docs/decision-request-real-tokio-interop-bridge.md`.

Supporting change evidence, cited by merged PR: #67 (affected-crate CI filtering), #68 (end-to-end check of that filtering), #69 (governance file set and first inspector report), #112 (inspector re-run correcting the HMAC cluster), #113–#117 (dependency swaps and the `rusty_retry`/`rusty_rsa` extractions), #122 (eleven-crate consolidation), #132 (interop bridge decision request), #133 (doctest platform exclusion), #134 (additive feature fix), #136 (explicit TLS provider). PR #131 is open and none of its content is in the evidence revision; cite it as a narrative reference at its own head SHA, not as part of the immutable record.

The record should support ATLAS-300 feature architecture, ATLAS-600 monorepo validation, ATLAS-001 maturity evidence, and ATLAS-100 mechanism/policy boundaries. Its limitations should say that repository evidence does not certify every crate, target, release, security property, or runtime environment, and that PR #131 is excluded. Per the ledger's maintenance rule, a later Atlas decision relying on #131 once merged needs a new dated record.

### Proposal C — non-normative monorepo practices note

Add a reference note, not a new normative chapter, describing:

- full-history subtree consolidation and its tradeoffs;
- scoped CI based on changed packages plus reverse dependencies under the CI feature graph;
- full sweeps for root manifest, lockfile, and CI-control changes;
- separate root and component ADR series with explicit remit;
- generated-map or map-validation approaches for rapidly changing workspaces;
- semantic review requirements for duplication/sovereignty tools, distinguishing clustering heuristics from verification patterns;
- mechanism-versus-policy extraction examples;
- stating executed versus compile-checked platforms per crate when consolidation changes the CI platform set.

### Proposal D — apply existing Atlas rules before adding more

Do not add new Atlas requirements for branch protection, required checks, PR rationale, topology-change review, or maturity non-transitivity. Atlas already covers these. Rusty Mill should implement the existing rules and then provide evidence about whether they are sufficient.

## 4. Rusty Mill Atlas-conformance review

This is an evidence-based alignment review, not a certification. Rusty Mill does not publish a formal conformance claim. `ARCHITECTURE.md` does cite ATLAS-300, but records that it was consulted when still a seed and deliberately not treated as a requirement; that statement predates ATLAS-300's promotion under Atlas ADR-0006 and is now itself stale.

### Conforming or strongly aligned

- The workspace boundary, explicit membership, resolver, local first-party resolution, and tracked lockfile align with core ATLAS-300 requirements.
- Architecture and ADR-0001 correctly state that co-location does not merge crate identity, runtime, release cadence, or per-crate governance.
- Shared dependency and metadata policy preserves justified divergence instead of forcing every crate into one false baseline.
- PR descriptions are unusually strong: they state what changed, why, risk, alternatives or exclusions, and concrete validation evidence. #134 in particular reproduces the original failure, proves the fix, and names the follow-up.
- CI includes formatting, Clippy with warnings denied, builds, tests, doctests, Linux/Windows execution, and explicit platform exclusions.
- Impact-aware selection includes transitive dependents and uses the same `--all-features` metadata graph as the selected validation.
- The project records negative evidence and deliberate non-actions instead of forcing every apparent duplicate into one abstraction.
- Per-crate documentation states which platforms have actually executed rather than inheriting upstream claims.

### Partial alignment or uncertainty

- `ARCHITECTURE.md` (97 lines) and the README (1,186 lines) provide a repository map, but the README is too large and already stale. A 174-member workspace needs a generated inventory or a validator that compares documented membership and relationships to Cargo metadata.
- Component responsibility is mostly expressed through crate boundaries and inherited per-crate docs, but there is no concise authority index showing which documents govern which major areas.
- The CI planning algorithm is important governance infrastructure but `affected_crates.py` has no unit tests. PR #68 added an end-to-end check that the filtering selects a crate, which is not the same thing. Its nested-crate ownership (longest path wins) and reverse-dependency traversal deserve regression tests.
- Documentation-only changes skip build, test, and Clippy, which is reasonable, but the root pipeline has no equivalent link, map, or semantic-drift checks for governing documentation.
- The real-Tokio interop bridge document is correctly marked proposed. `crates/rusty_tokio/docs/adr/` holds only a template; the crate's working convention is `docs/decision-request-*.md`. If the bridge is accepted, the decision should be recorded through that convention or by seeding the ADR series, consistent with `ATLAS-KNOW-0001`. Since #134 removed the CI blocker, the bridge is a standalone `rusty_tokio` capability question rather than a prerequisite for #131.
- PR #131 is a coherent migration, but 30,716 added lines, 115 files, 43 commits, and 436 claimed capabilities create a review-evidence concentration risk. The capability manifest improves traceability, yet the reactor failure and the current Windows timeouts show that manifest closure must not be reported as runtime or system verification.

### Confirmed nonconformance if Rusty Mill is treated as an Atlas repository

1. **Default branch is unprotected.** GitHub reports `main` as `protected: false`, with required-status enforcement off. This conflicts with `ATLAS-TOOL-0010` and prevents enforcement of `ATLAS-TOOL-0011` and `0050`. It also leaves `CONTRIBUTING.md`'s "no direct pushes to the default branch" rule unenforced.
2. **Local review policy is not enforced.** `CONTRIBUTING.md` requires at least one approval, but every PR merged on 2026-09-02 (#132 through #136) was authored and merged by the same account with no submitted review. The unprotected branch permits this drift.
3. **Independent-review evidence is overstated or absent.** Recent merged PRs have CI and author rationale but no recorded independent review. Under `ATLAS-GOV-REVIEW-0061`/`0064`, those mechanisms must not be conflated; if independent review is unavailable, its absence should be stated.
4. **Major topology changes do not identify continuing Atlas obligations.** Consolidation evidence is strong, but PRs and ADR-0001 do not explicitly crosswalk ATLAS-100, 200, 300, and 600 as required by `ATLAS-TOOL-0300` for an Atlas-governed topology change.
5. **Repository map contains stale assertions.** The README contradicts current membership and dependency state, and contradicts itself on `rusty_simd`; `ARCHITECTURE.md` describes ATLAS-300 in a state that no longer exists. Both weaken `ATLAS-TOOL-0240` compliance.

## 5. Recommended action sequence

1. **Atlas:** add a `DOC-nnn` finding to `docs-audit.md` recording that ATLAS-300's feature-architecture trigger has fired, citing #134 and #136.
2. **Atlas:** add the immutable Rusty Mill evidence record to `evidence-provenance.md` before drafting normative text.
3. **Atlas:** draft one focused RFC/PR activating only feature-flag architecture, led by the additivity rule; do not broaden it into publishing, build profiles, unsafe Rust, or cross-compilation.
4. **Rusty Mill:** protect `main`, require the CI checks, block force-push and deletion, and decide whether one approval remains mandatory or whether the documented policy should explicitly allow author self-review when independent review is unavailable.
5. **Rusty Mill:** replace or validate the hand-maintained repository map using Cargo metadata, correct the ATLAS-300 reference in `ARCHITECTURE.md`, and add unit tests for affected-crate planning.
6. **Rusty Mill:** root-cause the Windows timeouts on #131 and land it, then decide the real-Tokio bridge on its own merits through the crate's decision-request convention. Do not represent capability-manifest closure as system verification.
7. **Atlas:** after Rusty Mill applies the current rules, re-audit whether any remaining pain reflects a true Atlas gap rather than local adoption debt.

## Overall disposition

**Atlas:** Redirect — activate the narrow, evidence-backed ATLAS-300 feature-strategy work, anchored on feature additivity.
**Rusty Mill:** Continue with governance corrections — its architecture and CI provide valuable evidence, but branch enforcement, review evidence, and repository-map freshness need correction before a conformance claim.

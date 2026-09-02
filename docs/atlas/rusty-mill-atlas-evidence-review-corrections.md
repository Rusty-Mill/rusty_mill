# Corrections applied to the Rusty Mill → Atlas Evidence Review (revision 1 → revision 2)

Every item below was verified on 2026-09-02 against Rusty Mill `06ca8669` (local checkout), the GitHub PR/branch state, and Atlas `390d6b0f` (shallow clone). All 39 Atlas requirement IDs cited in the assessment resolve to real headings in Atlas; none are mis-cited.

## A. Factual corrections (must fix before the assessment is used)

### A1. The feature-unification failure was already fixed at the reviewed revision
- PR #134 ("rusty_request: make the `tokio` feature additive") merged at 11:45 UTC on 2026-09-02 as commit `9c33680`, which is an ancestor of `06ca8669`. It replaces the compile-time backend swap with per-call runtime detection (`src/rt.rs`, `RawStream` enum), keeps all 59 `rusty_tokio` integration tests running with the feature on, and adds a regression test for the unification scenario.
- The assessment presents the defect as current: gap-table row 3 ("the unified feature changed the required executor/reactor without a compile-time contract failure"), lesson 1, and action item 6. Rewrite these in the past tense and state that the fix landed before the evidence revision.
- Proposal B omits #134 entirely. It is the single most important evidence PR for the ATLAS-300 amendment because it shows the resolution chosen: restore feature additivity, not a bridge, not a compile-time incompatibility error.
- Action item 6 ("resolve the proposed real-Tokio bridge ... then update PR #131") is stale. #134's own body says the bridge decision request (#132) "stays relevant as a general `rusty_tokio` capability, but this PR removes the CI blocker it was written against." PR #131 already merged `main` to pick up the fix. The bridge is no longer on #131's critical path.

### A2. The failure did not occur in the 174-member workspace
- `crates/rusty_meshed` and `crates/rusty_kafka` do not exist at `06ca8669`. The failing consumers were the nine crates PR #131 adds, on that PR's branch CI. PR #131 is still open with `mergeable_state: unstable`.
- Executive conclusion wording "Rusty Mill's 174-member workspace produced a real, repeatable integration failure" should read: the failure surfaced on PR #131's branch, which adds nine `rusty_tokio`-based crates to the workspace, when the full-workspace `--all-features` sweep unified `rusty_request`'s feature.
- Proposal B cannot list #131 as evidence "at revision `06ca8669`" because none of its content is in that revision. Cite it as an open PR at its own head SHA (`fc212774`) or move it to the non-normative narrative.

### A3. PR #131's current CI state differs from what the assessment implies
- After #134 and #136 were merged into it, the Ubuntu test leg passes. The Windows test leg fails: two `rusty-meshed-cli::metrics` tests time out at 600 s on all three nextest tries, and 6,265 of 10,190 tests were cancelled by fail-fast. Clippy on both OSes, fmt, plan, and the mingw check pass.
- The "manifest complete, runtime failed" narrative (lesson 4, crosswalk row 11) still holds, but the live evidence is now a Windows-only hang, not the reactor panic. Update the narrative so that a reader checking the PR today finds what the assessment describes.

### A4. Proposal B lists the wrong PR and omits the relevant ones
- #123 is "Fix rusty_homelab_mcp: wrap JSON passthrough tools so outputSchema is valid". It has nothing to do with consolidation, CI, or the inspector. It was almost certainly meant to be #112 ("repo-inspector: re-run confirms workspace unchanged, fixes row-folded cluster"), which is the HMAC correction the assessment relies on for lesson 7.
- Add: #67 (introduces affected-crate CI filtering, rust-cache, nextest), #68 (round-trip test that verifies the affected-crate filtering end to end), #69 (governance file set plus first repo-inspector report), #112, #133 (doctest step honours `windows-exclude`, evidence for ATLAS-TOOL-0180/0220), #134 (the fix), #136 (explicit rustls provider in tests, a second `--all-features` unification defect: both `ring` and `aws-lc-rs` compiled in).
- #136 is worth calling out on its own: it is a second, independent instance of the same class of failure (feature unification of a third-party crate changed test-time behaviour), which strengthens the "trigger fired" argument more than a single `rusty_request` case does.

### A5. Lesson 9 misattributes the macOS/OpenBSD execution evidence
- `crates/rusty_tokio/README.md` states: "This crate's macOS/BSD integrations have never run on real hardware; Windows now has, once." Its BSD paths are verified only by `cargo check` against Apple/FreeBSD/NetBSD targets, and OpenBSD/DragonFly cannot even be checked in that sandbox.
- The two real AF_UNIX defects (one Darwin, one OpenBSD) were caught by `rustils`' `platform-bsd` upstream CI (macOS natively, FreeBSD/OpenBSD in VMs) before consolidation. `rusty_mill`'s own CI has no macOS or BSD leg.
- Reframe the lesson as an ATLAS-TOOL-0180 example: the repository explicitly distinguishes executed platform evidence from compile-only evidence, and records that the monorepo's CI has narrowed the executed platform set relative to the upstream crate's. That is a stronger and accurate point.

### A6. "Rusty Mill does not identify an approved Atlas baseline" is only half true
- `ARCHITECTURE.md` (Structure section) explicitly cites ATLAS-300, explains it was consulted, and declines to treat it as a requirement because it was "still a seed/draft" that deferred workspace layout.
- That statement is itself stale: ATLAS-300 is active with the RWC requirement set via Atlas ADR-0006. Add this to the repository-map staleness finding (section 4, "Partial alignment") as a concrete, checkable claim, and note that the root document's Atlas reference predates Atlas's own promotion of the volume.

### A7. Section 4 action 6 assumes an ADR series that does not exist
- `crates/rusty_tokio/docs/adr/` contains only `0001-template.md`. The crate's actual convention is `docs/decision-request-*.md`, which the bridge document says explicitly. The recommendation should be "record the decision through the crate's decision-request convention or seed its ADR series", not "record it in the crate's ADR series".

## B. Analytical improvements (should fix)

### B1. Proposal A normalises the wrong thing
- Cargo's own reference already requires features to be additive. The Rusty Mill defect was a violation of that existing norm, and #134 fixed it by restoring additivity, not by documenting a stress profile or adding a bridge.
- As drafted, Proposal A item 3 ("All-features evidence is bounded ... must not be represented as proof of every supported deployment profile") reads as accommodating non-additive features. Lead the chapter with "features MUST be additive; `--all-features` is the validation of additivity", then keep the profile and runtime-precondition requirements as the rules for the residual cases (mutually exclusive TLS providers, allocators) where additivity is impossible.
- Item 7 lists two options (compile-time error, or documented runtime failure plus bridge). Add the third and preferred one demonstrated by #134: runtime dispatch that keeps the feature additive.

### B2. The trigger claim needs the trigger text and a candid weakness
- ATLAS-300's deferred row reads: "Multiple supported feature combinations create real compatibility, dependency, or validation differences that require a shared policy." Quote it.
- An Atlas reviewer can argue the trigger is only partly met: each incident was one package's defect, fixed locally, and no shared policy was needed to fix it. The assessment should meet that objection directly. The stronger argument is the pair of incidents (#134 `rusty_request`, #136 rustls providers) plus the CI design decision that the impact-analysis graph must match the validation graph (`affected_crates.py` docstring). That is a workspace-level policy, not a package fix.

### B3. Some crosswalk mappings are loose
- ATLAS-ARCH-0020 ("Orchestration Coordinates, Domains Decide") is a stretch for the retry extraction. ATLAS-PHIL-0101 alone carries that row.
- ATLAS-GOV-REVIEW-0020 (Program-Integrity Review) applies to "an Atlas project governed by a First Release Definition". Rusty Mill has none. Either drop it from row 11 or cite it as the control that would have caught the manifest-versus-runtime gap if one existed.
- ATLAS-SPEC-0020 (Parent Authority Readiness) fits only if the assessment names the unresolved parent: the `rusty_tokio` real-tokio interop decision request. Say so, or rely on ATLAS-LIFE-0021 alone, which is the exact fit.
- Row 5 says the root architecture document distinguishes "repository, Cargo-workspace, product, runtime, and domain boundaries". `ARCHITECTURE.md` distinguishes build/governance boundary from runtime and domain model and gives a layering table. It does not separate repository from Cargo workspace (it treats them as one) and never uses "product". Soften to what it actually says.

### B4. Lesson 7 conflates two distinct failures
- The cluster tool (`find_clusters.py`) matched on module path only; the human verification then used a substring grep (`fn hmac`) that also matched `fn hmac_md5` and `fn hmac_sha1`. The report records both. Stating them separately makes the "semantic review of duplication tools" recommendation in Proposal C more precise.

### B5. Proposal B should follow Atlas's existing ledger format
- `docs/reference/evidence-provenance.md` uses `EVID-<SOURCE>-<date>` records with a four-field table (source repository, immutable revision, verified date, Atlas decisions supported), blob links pinned to the revision, and a limitations paragraph. Draft the Rusty Mill record in that shape (for example `EVID-RUSTYMILL-2026-09-02`) so it can be pasted in.
- Atlas's maintenance rule requires a new dated record when a later decision relies on materially changed evidence. Since #131 is still moving, the record should be scoped to what is in `06ca8669` and say that #131 is excluded.
- Action item 1 ("create a documentation-audit finding") should point at Atlas's existing `docs-audit.md` and its `DOC-nnn` finding format.

### B6. Strengthen the repository-map staleness finding with the specifics found
- README says "`rusty_simd` is the one still outstanding" while the same file elsewhere says `rusty_simd` was "already merged", and `Cargo.toml` lists it as a member. The README is internally inconsistent, not only out of date.
- README says `rusty_tokio` "doesn't get depended on by any of the other crates in this repo". Twelve workspace packages now depend on it by path per `cargo metadata --all-features`: `rusty_boot`, `rusty_request`, `rusty_stream`, `rusty_voice`, `rusty-db-core`, `agentgateway-tls`, `sessionmgr-daemon`, `sessionmgr-proc`, `sessionmgr-tui`, and optionally `rusty_http`, `rusty_lsp`, `rusty_tls`. (The first version of this list said nine and included `rusty_proxmox`, which only mentions `rusty_tokio` in a manifest comment; corrected here.)
- README is 1,186 lines; ARCHITECTURE.md is 97.

### B7. Independent-review finding: widen the sample statement
- Zero submitted reviews on #132 and #136 is confirmed. The same is true of #134 and #135, and every PR in the window is authored and merged by the same account. Saying "recent merged PRs" is accurate; the assessment can say the sample was all of the PRs merged on 2026-09-02, which is a stronger basis than two.
- `CONTRIBUTING.md` also says "no direct pushes to the default branch". Branch protection off means that rule is likewise unenforced; add it to nonconformance item 1.

All items above are incorporated in revision 2 of the review, `rusty-mill-atlas-evidence-review.md` in this directory.

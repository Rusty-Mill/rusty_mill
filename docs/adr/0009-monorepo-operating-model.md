# ADR-0009: Monorepo operating model

Status: Proposed
Date: 2026-10-08

## Context

The consolidated best-practices research (ChatGPT, Gemini, Grok; 200+ crate
Rust monorepos on GitHub) was reviewed against this repository (~290
manifests, 239+ workspace members). Most of its advice is already in place
under earlier ADRs. This ADR fixes the model in one place, records where we
deliberately differ from the research, and lists the remaining gaps.

Guiding principle (from the research, kept): **shared source does not imply
shared cadence.** Developers work package-scoped, CI validates the affected
closure, products release independently.

## Decision

### 1. Structure — keep (ADR-0001, 0002, 0003)

- One virtual workspace, one root `Cargo.lock`, merge-commit history.
- Layers `foundation → platform → libs → apps → tools`, enforced by
  `check_workspace_layers.py`. No upward edges, no app→app edges.
- Shared dependency versions in `[workspace.dependencies]`; first-party
  crates resolve by path (`check_workspace_deps.py`).
- Dependency tiers per ADR-0002. A new third-party dependency is justified
  in its PR.
- No catch-all `common`/`utils` crate. A shared need gets a named,
  single-purpose foundation crate (`rusty_hex`, `rusty_percent`, …).

### 2. Change flow — keep, with one gate

- Trunk: short-lived branch → PR → `main`. No direct pushes.
- **Merge with a merge commit** (CONTRIBUTING). This deliberately departs
  from the research's squash recommendation: subtree history is the
  point of ADR-0001.
- CI validates the affected closure (`ci_plan.py`, `affected_crates.py`),
  queued per component on `main` (ADR-0006). **Unknown impact escalates to
  the full sweep, never down.** This rule is load-bearing; keep it when
  editing the planner.
- Escalation inputs that always force a broad run: toolchain, `Cargo.lock`
  with unprovable impact, non-additive root `Cargo.toml` edits, CI files.
- Branch protection requires **one** stable check, `required-gate`, an
  `if: always()` job that `needs:` every CI job and fails on any failure or
  cancellation (skipped-by-plan counts as pass). Individual dynamic matrix
  jobs are never required checks.
- Review: CONTRIBUTING's self-review rule stands. The *high-impact paths*
  below always wait for an independent reviewer when one exists.

High-impact paths: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
`.cargo/`, `deny.toml`, `.github/`, `docs/adr/`, `crates/foundation/**`.

### 3. Releases — keep (ADR-0004)

| Class | Rule |
| --- | --- |
| Product (ships binaries/plugins) | Own workflow, tag `<product>-vX.Y.Z`, own `RELEASE_NOTES.md`, version = its crates' `[package].version`, `make_latest: false`. Never blocked by another product's pipeline; concurrency group per product. |
| Public library (published to a registry) | Independent SemVer. Gains `cargo-semver-checks` in CI the day the first one is published. None is today. |
| Internal crate | Not published, no independent release; ships through its consumers. |

Tooling stays as is: version bump in a PR *is* the release trigger. We do
**not** adopt `release-plz`/`cargo-rail` release flows; they solve the
registry-publishing problem we don't have.

### 4. Developer loop

- Work package-scoped: `cargo check|test|clippy -p <crate>`; add dependents
  with `cargo tree -i <crate>` before a cross-crate change. Never run
  `--workspace` locally by habit.
- Use `git worktree` for parallel branches; each keeps its own `target/`
  and avoids mtime churn.
- rust-analyzer: set `linkedProjects` to the product you're in and
  `cargo.targetDir = "target/rust-analyzer"` so the editor doesn't hold the
  build lock. Documented, not committed (per-developer).
- Pin the toolchain in `rust-toolchain.toml`; CI reads the same file.

### 5. Security and supply chain

Required: Dependabot (npm + Cargo security, exists), `cargo-deny`
(advisories, licences, bans, sources) in CI, workflow `permissions:`
default `contents: read`, third-party actions pinned by commit SHA with
Dependabot's `github-actions` ecosystem keeping them current. Git
dependencies and `[patch]` entries need a PR-description justification.

### 6. Ownership

`.github/CODEOWNERS` lists the high-impact paths above plus each product
under `crates/apps/`. With one maintainer it routes review requests and
marks what *must* get an independent look; it is not a team structure. Add
teams only when a second regular maintainer appears.

### 7. Health signals (monthly, from Actions data, no new tooling)

Median and p90 required-CI time; share of PRs that took the full sweep;
flaky retries reported by nextest; open Dependabot alerts. A regression in
the first three is the trigger for the deferred items below.

### 8. Merge queue — adopted (supersedes the earlier deferral)

Decision: `main` is protected by a ruleset that requires `required-gate` and
the **GitHub merge queue**. Nobody force-merges around a red or stuck check;
a red check is fixed, or its flake is fixed (see `rp-server`'s usage-history
test), or the entry is re-queued.

Why now: the deferral's trigger, "PRs repeatedly rebased for staleness", was
met. Branches kept having `main` merged into them, each push cancelled the
run and restarted the 30-45 minute Windows jobs, and the pressure to merge
around CI was the symptom.

How CI supports it (this PR):
- `ci.yml` triggers on `merge_group` (`checks_requested`). It is the only
  workflow that feeds the gate, so it is the only one that needs the trigger;
  a test enforces both.
- `ci_plan.py` scopes a `merge_group` run against `merge_group.base_sha`
  (`--merge-base`), so a queue entry runs the affected closure, not the full
  sweep. No usable base falls back to the full sweep.
- Concurrency is event-isolated already: only `pull_request` runs share a
  per-PR key and cancel in progress; `merge_group` (like every other event)
  gets a `run-<id>` key, so a push to an open PR cannot abort a queue entry.
  A test pins this.

Status: the workflow side is in place and the ruleset below was applied on
2026-10-09 (a repository setting, not code); the first queued merge was this
change.

Ruleset to apply by hand (Settings > Rules > Rulesets > New branch ruleset).
Order matters: merge the PR that adds `merge_group` first, otherwise the
queue waits for a check that never starts.

- [ ] Name `main`; Enforcement **Active**; Target: default branch (`main`).
- [ ] **Bypass list: empty.** A break-glass is editing the ruleset, which
      leaves an audit trail; it is not a standing bypass.
- [ ] Restrict deletions; Block force pushes.
- [ ] Require linear history: **off** (merge commits are the policy).
- [ ] Require a pull request before merging; required approvals **0**
      (CONTRIBUTING allows self-review when no independent reviewer exists,
      and PRs opened from the maintainer's own account cannot be approved by
      it); Require review from Code Owners **off** for now; Allowed merge
      methods: **Merge** only.
- [ ] Require status checks to pass: add **`required-gate`** (GitHub Actions)
      and nothing else (matrix jobs are never required checks).
      "Require branches to be up to date before merging": **off** (the queue
      builds each entry on the latest `main` itself).
- [ ] **Require merge queue**, with:
  - [ ] Merge method: **Merge commit**.
  - [ ] Build concurrency: **3** (entries build speculatively in parallel).
        Each speculative build sits on top of the entries queued ahead of it,
        so a later entry's CI also covers the earlier PRs' changes.
  - [ ] Minimum group size: **1**; Maximum group size: **1**. This sets how
        many PRs are merged together at the end (no batching). It does **not**
        isolate what a queue run tests: with build concurrency above 1, a run's changed-file list
        spans everything between its event base and its head.
  - [ ] Wait time to meet minimum group size: leave the default (unused at 1).
  - [ ] Status check timeout: **120 minutes** (the default 60 is inside the
        full-sweep Windows runtime).
  - [ ] "Only merge non-failing pull requests".
- [ ] Repository Settings > General > Pull Requests: Allow merge commits
      **on**; squash and rebase **off** (matches CONTRIBUTING).

Check after applying: with an empty queue, open a trivial docs PR, "Merge
when ready", and confirm a `ci.yml` run for `merge_group` appears, the `plan`
job's changed files are that PR's (with other PRs already queued, expect their
files too), `required-gate` passes, and the result is a merge commit on
`main`.
Rollback: turn off "Require merge queue" in the ruleset (queued entries are
dropped, nothing merges), or set the ruleset to Disabled temporarily.

Known limits, revisit on evidence: the push-to-`main` run after each queue
merge repeats work the queue already validated (trim only if p90 CI time or
queue length regresses); batching above 1 only once Windows flakes are gone
and queue wait is the bottleneck; availability depends on the repository being
public in an organisation (the option must appear in the ruleset).

## Deliberately not adopted (revisit on the stated trigger)

| Item | Why not now | Trigger |
| --- | --- | --- |
| `cargo-hakari` workspace-hack | Few shared external deps; adds a crate every manifest must reference | Measured feature-thrash rebuilds, or external deps grow |
| `mold` in CI | Rust 1.90+ already links with `lld` on x86_64 Linux; the slow jobs are Windows | A Linux link step shows up in job timings |
| `sccache` beyond the trial | Trial only on the Windows test shards (`setup-build-env` input `sccache`); Linux keeps Swatinem's cache | Trial shows a warm-run win on Windows shards (compare against the 30-45 min baseline) and a stable hit rate |
| Resolver 3 / edition 2024 workspace-wide | Members carry mixed editions on purpose (see root `Cargo.toml` notes) | Own PR with broad validation, per crate family |
| Bazel, `cargo-rail`, "monorepo council" | Scale and team size don't justify | Never without a new ADR |
| Crate-catalog YAML | `generate_workspace_map.py` already emits a map; ownership lives in CODEOWNERS | Need for tier/owner data that CODEOWNERS can't carry |

## Gaps to close (from this review)

Verified against `main` at `a402fff`.

| # | Gap | Evidence | Size |
| --- | --- | --- | --- |
| 1 | No single required check | `ci.yml` has dynamic per-component jobs, no aggregator | S |
| 2 | Workflow tokens not least-privilege | `ci.yml`/`baseline.yml` have no `permissions:` | S |
| 3 | Toolchain declared twice, no `rust-toolchain.toml` | `RUST_TOOLCHAIN: "1.98.1"` in `ci.yml` and `baseline.yml` | S |
| 4 | No CODEOWNERS | `.github/CODEOWNERS` absent | S |
| 5 | No dependency policy gate | no `deny.toml`, no advisory/licence job; Dependabot alerts only | M |
| 6 | Actions pinned by tag | 64 `uses: …@v*`, 0 SHA pins; no `github-actions` Dependabot entry | M |
| 7 | No scheduled full sweep | no `schedule:` trigger; full sweep only on fallback/dispatch | S |
| 8 | Docs scattered | operating rules live across CONTRIBUTING, ARCHITECTURE, ADR-0003/4/6 | S (this ADR + pointers) |

## Adoption order

1. **Hygiene (all S):** 1, 2, 3, 4, 7 — done together in the PR that
   introduced `required-gate`, `permissions:`, `rust-toolchain.toml`,
   `CODEOWNERS` and the weekly `schedule:` sweep.
2. **Supply chain:** 6 (done: every action SHA-pinned, Dependabot
   `github-actions` group, test enforces it) and 5 (done warn-only: `deny.toml`
   plus a non-blocking `cargo-deny` job). Drop `continue-on-error` once the
   first findings are triaged.
3. **On trigger only:** the deferred table above.

Branch-protection changes (making `required-gate` the sole required check)
are a repository setting, not code — apply after gap 1 merges.

## Consequences

- One page answers "how do I change, review, and release here".
- Four small governance files appear (`CODEOWNERS`, `rust-toolchain.toml`,
  `deny.toml`, the `required-gate` job); no new services or crates.
- We knowingly keep merge commits and the Python planner instead of the
  research's squash + Rust `ci-impact` tool. Rewriting a working,
  tested planner for stylistic uniformity is not worth the risk.

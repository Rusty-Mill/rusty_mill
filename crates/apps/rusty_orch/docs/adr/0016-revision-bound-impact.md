# ADR-0016: Revision-bound proposed-change impact

- **Status:** Accepted for Stage 1
- **Date:** 2026-10-20

## Decision

Extend the existing read-only `inspect` interface with `--impact request.json`.
Compose the existing repository intelligence and CI reverse-dependency helper;
do not introduce a graph engine, crate, execution policy or task lifecycle.
Full Cargo identities are primary keys; names are accepted only when unique.
Resolved evidence retains aliases, dependency kinds and target conditions.

Join manifest paths to Millboard's exported repository/project ownership. Keep
its name-ordered containing-path rule and repository fallback. A project can
own multiple paths, a repository many projects, and the registry many repositories.
App families without explicit project ownership are labeled inferred and broaden
validation. Projects without Cargo members require conservative compatibility
validation. A missing Cargo owner or unknown requirement scope broadens the
whole workspace validation scope rather than certifying an exclusion.

Use RemindMe's existing engine-backed complete exporter, exposed as the additive
`impact-export [QUERY]` command through its existing daemon operation seam.
No filters, ranking limits, active-project or memory lifecycle rules truncate
that inventory. Optional existing retrieval provides supplemental context only.
Explicit `metadata.factory_requirement` scopes establish binding requirements;
ordinary retrieved prose cannot become a requirement or verified dependency.
Withdrawal is explicit (`binding: false`); archival, expiry and supersession
remain visible and do not silently remove a contract.

Reports carry the clean Git source revision, input/metadata content identities,
and every recorded requirement's updated-at revision plus content fingerprint.
A previous report detects added, removed, withdrawn, moved and edited contracts,
including contracts outside the selected impact. Inputs and source revision are
checked again before stdout is emitted. Dirty or changing source, incomplete
snapshots, malformed requirements and unavailable metadata fail with no report.

## Consequences

The artifact is a snapshot-bound, task-sized validation input, not live-service
freshness, permission to execute, or proof of green tests. Operators refresh
exports before using a report. Snapshots and reports live outside the clean
checkout. No database is opened by Orch, avoiding migrations, access updates or
new cross-family app dependencies. RemindMe's explicit export uses its existing
store/daemon lifecycle, and an optional query has ordinary search access effects.
Millboard's export exits before opening any store, credentials or services.

Python isolation, bundled first-party code, fixed argv, process deadlines and
output limits from ADR-0014 are retained. Goal execution, BBP hosting, sandboxing,
approvals and publication behavior are unchanged. No new persistence format or
migration is introduced in Rusty Multimodal DB.

The bundled program is sent through the existing process seam's stdin to
`python3 -I - <repo> [request]`. Passing its growing source in `-c` exceeded
Windows' CreateProcess command-line limit (error 206). Stdin script mode avoids
that limit while preserving isolated imports and avoiding temporary script files.

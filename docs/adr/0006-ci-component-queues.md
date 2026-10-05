# ADR-0006: Queue main validation by component and check

**Status:** Proposed (implemented locally for review)
**Date:** 2026-10-05
**Deciders:** Repository owner
**Issue:** [#264](https://github.com/Rusty-Mill/rusty_mill/issues/264)

## Context

The affected-crate planner uses a verified push `before..HEAD` range and
transitive reverse dependencies. The workflow-wide `ci-main` concurrency group
nevertheless prevents unrelated applications from even starting their planners.
Its default single pending slot can replace a pending application A run with
an application B run whose selected tests do not include A. Protecting an active
run from cancellation does not protect pending coverage.

## Decision

Preserve per-push validation at each event's exact SHA. Main and manual workflow
runs receive unique groups so their planners and global checks start independently
within GitHub runner limits. Revisions of one PR retain workflow-level cancellation.

Partition the existing affected package set into stable components: each directory
under `crates/apps` owns its nested packages; libraries share category lanes and
foundation crates share a foundation lane. This partition does not add or remove
packages. Shared-library changes still fan out through the Cargo reverse dependency
graph into every affected application, and path-based product checks stay enabled.

Queue each main component/check/OS/shard combination independently. Dedicated
product jobs have their own stable check keys, including the feature dimension
where present. Every such job uses literal `queue: max` and
`cancel-in-progress: false`. Non-main jobs use run-specific keys, separate from
these main queues. Global format, planner, workflow-lint and dependency-policy
jobs have no shared queue and still run for every SHA.

Full fallbacks and manual sweeps retain the existing workspace scope, three test
shards, per-OS exclusions, special feature suites, doctests and console tests.
Full-sweep jobs have separate workspace keys; hosted runner isolation allows them
to overlap scoped application jobs. Scoped matrices fall back to one combined
scope if more than 128 components would exceed the two-OS, 256-job matrix limit.
All selected packages remain in that fallback.

## Options considered

- **One main queue with `queue: max`:** retains pending work but still makes
  unrelated applications wait for the slowest application.
- **Coalesce per-application work:** could validate only the latest state, but
  would require a durable last-successful validation frontier and cumulative
  impact calculation. A later single-push diff is not sufficient, even within
  one application containing multiple crates.
- **Independent component/check queues:** chosen; retains exact-SHA evidence,
  uses the existing impact planner and allows unrelated applications to proceed.

## Validation and limitations

[GitHub's documented concurrency syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#concurrency)
allows up to 100 pending jobs with `queue: max`; additional arrivals are canceled
when that queue is full. Ordering follows arrival at the concurrency group, not
commit order. Therefore no individual green result is presented as evidence for
the latest main SHA without checking that SHA. This is not an unlimited queue.

Pinned actionlint 1.7.12, the latest official release checked, does not recognize
`queue` ([upstream #657](https://github.com/rhysd/actionlint/issues/657)).
`lint_workflows.py` runs it on unchanged source and adds a narrow validation rule:
only a literal `queue: max` paired with explicit `cancel-in-progress: false` is
accepted. Only the exact unsupported-key diagnostic at that validated line is
handled by the extension. Invalid values, cancellation combinations and all other
syntax/expression errors fail. Remove the adapter when upstream supports the key.
The extension requires complete single-line scalars and whitespace before comments;
real pinned-actionlint regression probes run in the workflow-lint job.

Component and workflow tests cover separate app pushes, differing package subsets
in repeated same-app pushes, shared dependency fanout, global fallback coverage,
unique artifact/cache names and preservation of test settings. Local validation
does not replace a hosted exact-head CI run of the changed workflow.

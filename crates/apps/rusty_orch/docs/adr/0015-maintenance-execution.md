# ADR-0015: Opt-in maintenance uses the existing BBP task host

- **Status:** Accepted for Milestone 2 implementation
- **Date:** 2026-10-18

`maintain` consumes the same versioned inspection produced by `inspect`, at the
command composition root. It selects one metadata repair or explicitly beneficial
dependency-declaration hoist, with at most two affected crates and eight crates in
the conservative reverse closure. Structural repairs remain excluded. Ranking is
policy repair first, then validation scope, edit scope, and portable finding ID.

Maintenance uses `orch-dispatch::TaskHost` and `orch-bbp::BbpHost` directly for its
single Implement card. It does not route a one-call implementation agent: all
providers, capabilities, writers and independent reviewers come from the supplied
BBP configuration. Built-in tools are disabled. Existing moderator supervision,
Linux confinement, budgets and human gates remain authoritative.

A private atomic JSON journal under the common Git directory records intent
before opening BBP, immutable configuration/profile bytes, the portable finding
identity, base revision, handle, public task observations, outcomes and PR URL.
The common directory lock serializes linked-worktree maintenance invocations.
The stable namespace lets BBP adopt an open whose reply/checkpoint was lost.
There is no new database or task protocol. Completed/cancelled findings are never
selected again; interrupted work resumes the same task. Prepared work remains
active until a human closes/cancels BBP, so publication never frees the slot.

A bundled Python scope profile verifies the staged candidate's paths and TOML
semantics inside the existing BBP sandbox before Cargo validation. It permits
only regular manifest modifications and preserves dependency versions/features.
The remaining frozen profiles include fmt plus tests and clippy for affected
packages and reverse dependencies, using frozen all-feature Cargo resolution.

At the BBP merge gate, maintenance verifies the current candidate/report, passing
scope and Cargo profiles, sandbox evidence and inspected base, then prepares a
patch, PR description and report. Explicit `--execute --publish-pr` authorizes
publication of that exact validated tree from a private worktree as a draft PR.
A stable branch and current PR head check recover uncertain push/create replies.
Maintenance never relays approvals, grants a merge gate or merges a PR. The
existing human CLI and receipt lifecycle remain unchanged.

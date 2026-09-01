# ADR-0002: Reporting surface stays JSON-only, not a dashboard

Status: Superseded by [ADR-0003](./0003-minimal-static-dashboard.md)
Date: 2026-08-06

## Context

A comparison against OmniRoute (a much larger TypeScript "AI gateway"
product — Electron desktop app, PWA, web dashboard, 291 providers) prompted
a parity effort: close the capability gap where it's additive, without
chasing OmniRoute's literal product surface. `ARCHITECTURE.md` previously
listed "not a full LLM gateway UI/analytics product, no dashboard" as a hard
non-goal, which would have blocked even the additive parts of that effort
(e.g. a free-tier budget report, à la OmniRoute's `/dashboard/free-tiers`).

## Decision

Keep every new reporting surface (`GET /v1/free-tiers` and anything
similar added later) JSON-only, consumed via `curl`/Prometheus/a client's
own tooling — the same shape as the existing `GET /v1/usage`,
`GET /v1/providers/stats`, and `GET /metrics`. No HTML, no bundled frontend,
no Electron/PWA wrapper, no `dashboard` route. The non-goal changes from
"no reporting surface at all" to "no *UI* — reporting stays an API."

## Alternatives considered

- **Ship a bundled web dashboard.** Rejected: pulls in a whole frontend
  toolchain into a single-binary Rust project whose entire value
  proposition is being a small, auditable, single-artifact deploy. Nothing
  about rusty_provider's operator base (people running `cargo run -p
  rp-server` or a Docker image) asked for a UI; they read `/metrics` and
  `/v1/usage` today.
- **Full non-goal removal (chase OmniRoute's literal product surface).**
  Rejected as this run's scope — see the user's parity-loop scoping
  decision: explicitly no Electron app, no PWA, no i18n, no MITM-based
  third-party CLI config injection.

## Consequences

- New operator-facing data (free-tier budgets, future stats) is always a
  new `GET` JSON endpoint plus a `RELEASE_NOTES.md`/README entry, never a
  new frontend dependency.
- A future decision to add an actual UI is still open, but it now needs its
  own ADR superseding this one — it's not a silent scope creep one endpoint
  at a time.
- `rp-server` stays dependency-light: no templating engine, no static asset
  bundling, no JS build step in the Rust build.

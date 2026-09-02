# ADR-0003: Add a minimal static dashboard, superseding ADR-0002

Status: Accepted
Date: 2026-08-07
Supersedes: [ADR-0002](./0002-reporting-surface-is-json-only.md)

## Context

ADR-0002 kept every reporting surface JSON-only and left "a future decision
to add an actual UI... needs its own ADR superseding this one." That
decision arrived: an operator asked what it would take to visualize the
JSON endpoints that already exist (`/v1/models`, `/v1/usage`,
`/v1/providers/stats`, `/v1/free-tiers`, `/v1/admin/clients*`) rather than
reading them via `curl`.

## Decision

Add `GET /dashboard`: one static HTML file with vanilla JS, no build step,
no npm, no JS framework, no CDN dependency, compiled into the `rp-server`
binary via `include_str!` (`crates/server/assets/dashboard.html`). It
authenticates and renders entirely client-side — the page itself carries
no secrets and needs none to load, so it's served unauthenticated (same as
`/health`); its JS prompts for a bearer token and attaches it to every
`fetch()` call against the existing JSON endpoints, so it's subject to
exactly the same `check_auth`/`check_admin_auth` rules those endpoints
already enforce. No new data, no new persistence, no new auth model — a
rendering layer over what already existed.

`ARCHITECTURE.md`'s non-goal changes from "no *UI* -- reporting stays an
API" back to "no *desktop/Electron* product" -- the JSON API itself is
unaffected and stays the primary/canonical interface; the dashboard is
purely an optional, read-mostly view over it.

## Alternatives considered

- **Stay JSON-only (keep ADR-0002 as-is).** Rejected per the request that
  prompted this ADR — an operator specifically wanted a visual surface, not
  another `curl` invocation to remember.
- **A real SPA (React/Svelte + build pipeline).** Rejected for now: it
  would look more like OmniRoute's own dashboard, but drags an npm
  toolchain into a repo whose entire value proposition (per ADR-0002's own
  reasoning, which still holds) is being a small, auditable, single-binary
  deploy. Nothing here rules out revisiting this later if the vanilla-JS
  page's limits (styling, interactivity, bundle-as-one-file discipline)
  start to bite — that would be its own ADR, not a silent upgrade.
- **A separate binary/process serving the dashboard.** Rejected: it would
  need its own auth story or a proxy back to `rp-server`'s API, for no
  benefit over compiling one HTML file into the existing binary.

## Consequences

- `rp-server` gains exactly one new unauthenticated route (`GET
  /dashboard`) and one new file (`crates/server/assets/dashboard.html`);
  zero new Rust dependencies, zero new config surface, zero new persisted
  state.
- The dashboard can only ever show what the JSON API already exposes —
  there is deliberately no path for it to gain capabilities the API
  doesn't already have (e.g. no direct database access, no bypass of
  `check_auth`/`check_admin_auth`).
- A future move to a real SPA/build pipeline is still open, but — same
  discipline ADR-0002 established — it needs its own ADR, not incremental
  scope creep on top of this one.

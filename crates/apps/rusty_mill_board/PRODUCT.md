# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

One person, the board's owner (Nano), in a desktop web browser. The board runs on a home
server (ms-01) and is reached over an SSH tunnel at `http://127.0.0.1:8787`. Agents (Claude
sessions, builders, reviewers) use the same board through MCP, not through the web UI; the web
UI is where the owner watches and steers their work.

## Product Purpose

Mill Board is a capability-gated kanban board that humans and AI agents share. The web UI lets
the owner, in one place:

- glance at what is in flight and which agent holds it;
- triage the backlog (priority, project, links);
- approve gates and answer agent decisions;
- see progress per project (roadmap, metrics, archive);
- administer the board (agents, tokens, repositories, roles, columns, rules, GitHub, apps).

All five jobs matter (owner, 2026-11-05). Success is that the owner can see the state of 20+
projects and ~750 cards quickly and act without hunting.

## Positioning

The board is the only state for agent work, and it enforces who may move a card where: each move
needs a capability, some moves are human gates, and work columns require a claim. Agents and the
human share one board; the UI shows agent claims and live activity alongside human approvals.

## Operating Context

- Cards live in rusty_tick (ADR-0012); the board reads and writes them over HTTP.
- Scale today: ~757 cards (≈510 Done, ≈210 Backlog), 20+ projects across several repositories.
- Columns are admin data (`workflow.json`); the live board uses the factory workflow: Backlog,
  Ready, Planning, Architecting, Building, V&Ving, Done, Blocked. Column kinds: backlog, queue,
  work, done, blocked.
- Cards carry: number (#n), title, body, project, tags, priority (0/1/3/5), deps, related,
  superseded_by, claim, comments, execution link, created/updated/done times, archive state.
- Updates arrive live (board events polled by the UI).
- Signed-in credentials fix the actor and role.

## Capabilities and Constraints

- Served by the Rust binary as one HTML document (`src/ui.html`, `include_str!`); the API is
  `POST /api/<op>`. Any redesign keeps these API ops and their permission behaviour.
- Owner said everything about the UI may change (2026-11-05). Whether to keep a single
  self-contained file with no front-end build step is a recommendation, not a constraint: the
  repository prefers minimal dependencies.
- Browser checks live in `tests/ui/*.mjs` (Playwright, manual, not in CI); they select by ids and
  classes and must be updated with any markup change.

## Evidence on Hand

- Real board data via the board's own MCP `board_list` (757 cards on 2026-11-05).
- Owner screenshots of the current UI (board, roadmap, metrics, settings tabs), 2026-11-05.
- No brand assets, logo or typeface commitments exist.

## Product Principles

1. State at a glance: what is moving, who holds it, what waits on the owner.
2. The owner's attention is the scarce resource: gates and decisions surface first.
3. Rules are visible: a refused move says why; capabilities and gates are legible.
4. Scale without clutter: hundreds of cards and dozens of projects stay navigable.
5. Small and plain: no framework or service where a page and the existing API suffice.

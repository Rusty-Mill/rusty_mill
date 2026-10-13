---
name: project-template
description: AGENTS.md skeleton - scope, project shape, commands, conventions, boundaries, coordination.
tags: [template, agents]
---
# AGENTS.md

## Scope

What this file covers (crate/dir) and which parent conventions apply on top.

## Project shape

- Purpose: one sentence.
- Layout: where `src/`, `tests/`, docs and ADRs live.
- Boundaries: dependency direction, what must never import what.

## Commands

- Build: `<cmd>`
- Test: `<cmd>` (fast subset: `<cmd>`)
- Lint/format: `<cmd>`

## Conventions

- Style, error handling, naming - only what differs from the defaults.
- Never skip, disable or loosen a test to get green.

## Boundaries

- Do without asking: edits within scope, running the commands above.
- Ask first: public API changes, migrations, dependency bumps, deletions.
- Never: commit secrets, touch frozen or generated paths (list them).

## Coordination

Where handoffs, review rules and workflow docs live.

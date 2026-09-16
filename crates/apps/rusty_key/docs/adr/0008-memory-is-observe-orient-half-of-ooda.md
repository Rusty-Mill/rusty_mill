# ADR-0008: Memory is the Observe + Orient half of the OODA loop

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: memory, ooda, feed

## Context

The system needs a coherent mental model for how memory relates to the agent
loop. Keystone framed the harness in OODA terms; Rusty Keys carries that framing
forward so each memory operation has a clear role in the loop.

## Decision

Treat memory as the Observe + Orient half of the OODA loop. The short-term
stream captures every event (Observe); recall assembles working memory each turn
(Orient); the kernel is Decide + Act; outputs feed back as observations.

## Consequences

- Memory's mental model is deliberately coupled to OODA.
- Embraced intentionally as a unifying frame rather than treated as a constraint.

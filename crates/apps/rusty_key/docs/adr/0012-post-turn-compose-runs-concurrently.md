# ADR-0012: Post-turn compose runs concurrently

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: async, compose, concurrency

## Context

In Keystone the sequential post-turn LLM calls added visible latency between the
reply appearing and the next prompt. After the kernel returns a reply, the
criteria judge and idle consolidation are independent of one another and of the
reply already being available to the caller.

## Decision

Run the post-turn criteria judge and idle consolidation concurrently via
`tokio::join!` while the reply is already in the caller's hands, so both calls
overlap with the user reading the reply.

## Consequences

- If consolidation fires before the criteria judge completes, it may miss the
  judge's learning signal.
- Mitigated by joining both before observing their results.

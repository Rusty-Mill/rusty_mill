# ADR-0002: aisdk as LLM provider abstraction

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: llm, provider, integration

## Context

Keystone used LiteLLM plus a manually authored `Tool` dataclass and JSON schema.
Rusty Keys needs a provider-agnostic LLM layer that is native to the Rust async
runtime and removes the manual schema authorship burden. Model identity should
remain a configuration concern, not a code concern.

## Decision

Use [aisdk](https://aisdk.rs) as the LLM provider abstraction. It provides 73+
providers, native tokio async, streaming, structured output, and a `#[tool]`
proc macro that generates JSON schema from Rust function signatures — a complete
replacement for LiteLLM and Keystone's manual `Tool` struct. Model identity stays
a config string (`RUSTYKEYS_MODEL`); swapping providers requires no code changes.

## Consequences

- aisdk is newer than LiteLLM and has not accumulated the same production
  edge-case coverage.
- Watch for rough edges in provider normalization.

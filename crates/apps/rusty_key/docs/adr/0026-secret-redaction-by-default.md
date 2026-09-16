# ADR-0026: Secret redaction by default before logging, journaling, or emitting

- Status: Accepted
- Date: 2026-05-27
- Tags: security, redaction, observe

## Context

Tool arguments and results can contain secrets (API keys, tokens, credentials).
Today PRD 02 lists `RedactPolicy` only as a future seam, so secrets could reach
`.rustykeys/evidence.jsonl`, `.rustykeys/security.jsonl`, the `/evidence` view,
and `rk://tool_event` IPC/Tauri payloads unredacted (consolidated plan §H). For a
harness where the LLM is semi-trusted, leaking secrets into durable logs and the
desktop bridge is a real exposure.

## Decision

Redact secrets by default before any tool args or results are logged, journaled,
or emitted over IPC/HTTP. This promotes `RedactPolicy` from an optional seam to a
required default applied at the observe/emit boundary: a deny-list of argument
keys plus a value scrub run before anything hits the evidence/security logs,
`/evidence`, or `rk://tool_event`. The redaction rule and deny-list live in
`docs/architecture/threat-model.md`.

## Consequences

- Secrets do not reach durable logs or the desktop event bridge by default.
- Redaction runs in the emit path for every tool event, adding a scrub pass over
  args/results.
- Over-redaction is possible for fields that look like secrets but are not;
  tuned via the deny-list.

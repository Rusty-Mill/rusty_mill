# ADR-0020: An HTTP adapter for local model servers

- **Status:** Accepted (Let Em Loose #594, owner answers L2 and L4, 2026-11-02)
- **Date:** 2026-11-03

## Context

Every orch adapter so far drives a subscription CLI with no API key. The
lens runner needs local models served by llama-swap (Qwen3.6-35B-A3B and
gpt-oss-120b on BaileyAi), which speak the OpenAI chat-completions protocol
over HTTP, and Gemini later over the same protocol. The 2026-10-29
consolidation audit already counts seven blocking HTTP clients.

## Decision

- A new adapter crate, `orch-openai`, beside the CLI adapters.
- Plain HTTP/1.1 on the workspace's sans-IO `rusty_http`, over a blocking
  `std::net::TcpStream`. No new third-party dependency, and no eighth client:
  it is a request builder over the shared message layer. `https` is refused
  until Gemini needs it, which waits for the audit's canonical client.
- Loopback and private addresses only. The host is resolved on every call;
  every answer must be private, and the adapter connects to an address it
  checked, never the name, so DNS cannot change between check and connect.
- An optional API key, for the later Gemini endpoint, is the one exception
  to "no vendor API keys": named by environment variable, read at call time,
  never logged, masked out of errors. The local servers need none.
- The adapter returns the reply text and usage. Validating the reply belongs
  to the lens runner's shared parser, so every adapter is held to one rule.

## Consequences

- A sensitive claim can be pinned to a provider that cannot leave the LAN,
  checked by address rather than by trusting a hostname.
- A model server on a public address, or behind a name with a public
  answer, cannot be used until TLS support lands.
- An IPv6 literal URL needs an explicit port (a `rusty_http::Url` limit).

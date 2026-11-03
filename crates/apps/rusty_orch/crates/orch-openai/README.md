# orch-openai

One OpenAI-compatible chat completion (`POST {base_url}/chat/completions`)
over plain HTTP/1.1, for a model server on the local network. In v0 that is
llama-swap on BaileyAi; Gemini comes later over HTTPS. Part of the Let Em
Loose lens runner (#594), [ADR-0020](../../docs/adr/0020-openai-http-adapter.md).

```rust
let config = Config::new("http://baileyai:8080/v1", "Qwen3.6-35B-A3B", None, Duration::from_secs(120), true)?;
let reply = OpenAiHttp::new(config).complete(&Prompt { system, user, max_output_tokens: 2048 })?;
```

- `https://` is refused at config time.
- Every call resolves the host and refuses unless every address is
  loopback, RFC 1918 or `fc00::/7`; it then connects to an address it
  checked. A refusal is `Permanent` and names the address.
- One deadline bounds resolve, connect, send and read.
- Errors are `ClassifiedError`s: unreachable server, 401/403/404 or a missing
  key is `Unavailable` (try the next provider); timeout, 408, 429 or 5xx is
  `Transient`; anything else is `Permanent`.
- An optional `api_key_env` is read at call time, sent as a bearer token,
  and masked out of any error text. llama-swap needs none.

It returns the reply text and token usage; it does not interpret the reply.

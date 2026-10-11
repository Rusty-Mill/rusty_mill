---
category: Fixed
changelog: `rusty_request` skips interim `1xx` responses (`100 Continue`, `103 Early Hints`, ...) and returns the final response, instead of treating the first head it reads as the answer.
---
## 2026-10-11 - rusty_request: interim responses no longer mistaken for the answer

- **Fixed:** `send` and `send_streaming` read one response head and treated it as final, so a server that sent `103 Early Hints` (or `100 Continue`) ahead of its `200` made the caller see a `103` with no body. Interim `1xx` heads are now skipped (RFC 9110 §15.2) and the bytes that arrive after them are kept for the next read. `101 Switching Protocols` is still returned as the answer. More than 16 interim responses is an `Error::InvalidResponse`.
- **Found by:** moving the `rusty-mcp` JWKS fetch from `reqwest` (which handles this) to `rusty_request`.
- **Verified:** five new tests in `tests/client.rs` (one and several interim responses, the streaming path, the limit, and `101`); each of the three code paths fails its tests when reverted (checked).
- **Known limitations:** interim responses are discarded, so `Link` headers from `103 Early Hints` are not exposed to the caller. The `Expect: 100-continue` handshake itself is not implemented; this only stops a `100 Continue` from being mistaken for the final response.

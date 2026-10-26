---
category: Added
changelog: rusty_request pools streamed responses, drops idle connections the server closed, and adds header_append and Error::is_connect (#885).
---
## 2026-10-25 - rusty_request: what a reverse proxy needs from a client

- **Added:** `send_streaming` returns its connection to the pool once `StreamingResponse::chunk` has returned `None`, when the server left it open (no `Connection: close`, not close-delimited, not `101 Switching Protocols`, nothing past the body). Before, every streamed request dialled a new connection. Dropping the response before its end still closes the connection. A `101` connection is no longer pooled by `send` either.
- **Fixed:** a pooled connection the server closed while it sat idle is now noticed before reuse (one read that must find nothing waiting) and dropped, so the request goes out on a fresh connection instead of failing. A connection the server closes once the request is on its way still fails and is still never replayed.
- **Fixed:** `rusty_http`'s async `BodyReader::next_chunk` (both runtimes) keeps its chunked decoder in step with the bytes it consumed when the call is dropped mid-read. Before, a later call could read chunk data as framing and end the body early, which pooling would then have reused.
- **Added:** `RequestBuilder::header_append`, which keeps earlier values of the same name (`header` replaces them), and `Error::is_connect`, true when the request failed before it was sent: resolving, dialing, or the I/O of a `CONNECT` tunnel, on the first hop. It is never true once a redirect was followed, or once an earlier retry attempt may have delivered the request. The error's kind and message are unchanged.
- **Known limitations:** a TLS handshake failure is not a connect error, since the handshake runs with the first write; nor is a proxy refusing the tunnel (`Error::ProxyConnectFailed`). Intermediate redirect hops of a streamed request are still not pooled.

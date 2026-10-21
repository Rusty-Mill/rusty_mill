# rusty_hyper

HTTP/2-only client and server packaging over `rusty_h2`. `connect(url, options)`
opens a plain h2c connection (`http://`, prior knowledge) or TLS with ALPN `h2`
(`https://`, optionally with a client certificate); `serve(listener, tls,
options, handler)` accepts connections and runs a closure per request. Connect
and handshake timeouts, an optional keepalive, a connection cap and graceful
shutdown are built in. No HTTP/1.1, no proxies, no redirects, no `Upgrade`.

The transport is behind the `rusty-tokio` feature; a default build is empty.
It is the transport for the first-party gRPC stack (`rusty_tonic`).

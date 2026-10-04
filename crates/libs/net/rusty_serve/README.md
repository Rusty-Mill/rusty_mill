# rusty_serve

A small blocking HTTP/1.1 server on `rusty_http` for a JSON API plus a
built web UI: `std::net` sockets, one thread per connection, no async
runtime. Extracted from `rusty_tick`'s `server.rs` and `static_files.rs`
when `rusty_fair_play` needed the same two files, so the two apps share
one copy.

## Shape

- `Handler`: `fn handle(&mut self, &Request) -> Response`. A `Request` is
  the method, the origin-form target, the `Authorization` and `If-Match`
  header values and the body; a `Response` is a status and a JSON body
  (empty for 204). The handler never sees the transport, so every route
  is testable without a socket, and it runs under one lock, so it may
  hold `&mut` state.
- `Server::bind(addr, handler)`, `.with_web_dir(dir)`, `.run()`,
  `.shutdown_handle()`. `/api/...` and `/health` go to the handler;
  anything else serves the web directory through `static_files::load`
  (path-safe: no `..`, no symlink escape, unknown paths fall back to
  `index.html` so client-side routes survive a reload) with a strict
  `Content-Security-Policy`.
- Bounds: `MAX_BODY_BYTES` (1 MiB, 413), a 16 KiB head (431), a 30 s idle
  timeout and `DEFAULT_MAX_CONNECTIONS` (64, 503). Keep-alive is honoured;
  a poisoned handler lock answers a generic 500.

## Use

```rust
impl rusty_serve::Handler for MyApi {
    fn handle(&mut self, r: &rusty_serve::Request<'_>) -> rusty_serve::Response { /* route */ }
}
let server = rusty_serve::Server::bind(addr, api)?.with_web_dir("web/dist".into());
server.run()?;
```

Deliberately not async: a personal server has a handful of connections
and one store behind one lock, so there is no I/O concurrency for a
runtime to exploit. Users: `rusty_tick`, `rusty_fair_play`.

```
cargo test -p rusty_serve
```

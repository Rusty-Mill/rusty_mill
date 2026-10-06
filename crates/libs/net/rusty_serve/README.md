# rusty_serve

A small blocking HTTP/1.1 server on `rusty_http` for a JSON API plus a
built web UI: `std::net` sockets, one thread per connection, no async
runtime. Extracted from `rusty_tick`'s `server.rs` and `static_files.rs`
when `rusty_fair_play` needed the same two files, so the two apps share
one copy.

## Shape

- `Handler`: `fn handle(&mut self, &Request) -> Response`. A `Request` carries
  the method, target, body, the `Authorization` and `If-Match` values, and
  every header (`headers`, for a webhook's signature, say). A `Request` is
  the method, the origin-form target, the `Authorization` and `If-Match`
  header values and the body; a `Response` is a status and a `Body`:
  `Body::Json` (a document, empty for 204, built with `Response::json`)
  or `Body::Stream` (chunks from an iterator, built with
  `Response::stream`, sent as chunked transfer encoding after the
  handler's lock is released, for `text/event-stream` and the like) or
  `Body::Deferred` (a job that produces the status and body, built with
  `Response::deferred` and run after the lock is released, for a route that
  waits on something slow and must not hold up the others). The handler never sees the transport, so every route
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
runtime to exploit. Users: `rusty_tick`, `rusty_fair_play`, `rusty_agui`
(its `serve` feature streams AG-UI runs through `Body::Stream`).

```
cargo test -p rusty_serve
```

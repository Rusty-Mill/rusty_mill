# rusty_stream

The workspace's async `Stream` trait, its combinators and the `stream!` /
`try_stream!` macros. It is the in-house replacement for `futures`,
`futures-util`, `futures-core` and `async-stream` (#896, gap G7 in
`docs/research/ASYNC-RUNTIME-INVENTORY.md`).

The default build has no third-party dependencies and no `unsafe` code, and
it runs on any executor.

| `futures` / `async-stream` | `rusty_stream` |
|---|---|
| `Stream`, `StreamExt::{next, map, filter, filter_map, then, buffered, take, collect, boxed}` | same names |
| `TryStreamExt::{try_next, map_ok, map_err, try_collect}` | same names |
| `stream::{BoxStream, iter, once, unfold, select_all}` | `rusty_stream::{BoxStream, stream::*}` |
| `future::{BoxFuture, join_all, ready}` | `rusty_stream::future::*` |
| `channel::mpsc::unbounded` | `rusty_stream::channel::mpsc::unbounded` |
| `async_stream::{stream!, try_stream!}` | `rusty_stream::{stream!, try_stream!}` |

## Differences from `futures`

- **Adapters need `Unpin` streams.** This is what keeps the crate free of
  `unsafe`. Pin a `!Unpin` stream first with `.boxed()` or `Box::pin`. Streams
  from the macros and this crate's constructors are already `Unpin`. Adapters
  box the futures they hold (`then`, `filter`, `buffered`, ...).
- **`select_all` is round-robin.** Each poll starts one stream further along,
  so a stream that is always ready cannot starve the others.
- **`stream!` / `try_stream!` rewrite tokens, without `syn`.** Inside
  `try_stream!`, `?` yields the error as the last item and ends the stream,
  and a bare `return;` ends the stream early. A bare `return;` inside a nested
  `fn` item is not supported. Yields inside a nested macro call belong to
  that macro.

## Interop

The `futures-core` feature adds `Compat<S>`, which adapts this crate's
`Stream` to `futures_core::Stream` and back. Use it where a stream crosses into
axum SSE, reqwest's `bytes_stream`, tonic, or another crate that speaks
`futures_core::Stream`.

```bash
cargo test -p rusty_stream --all-features
```

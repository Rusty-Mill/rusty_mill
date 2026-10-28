# Spike 4: roaming transport

Question ([SPIKES.md](SPIKES.md) row 4, [RISKS.md](RISKS.md) R3): which transport carries a
remote session across a client IP change, UDP or QUIC, and where predictive echo hooks in.
Run 2026-10-28 on `main` at 7197c347. Report only: the prototype is not for merge.

## Answer

1. **First, TCP + `rusty_tls`, with reconnect-and-resume.** The protocol already
   survives a reconnect: `Hello { resume }` replays missed events (`Features::EVENT_SEQ`,
   `server/src/history.rs`), and attach re-sends the block's screen as a checkpoint. The
   prototype moved a client from 127.0.0.2 to 127.0.0.3 over TCP and got the event stream
   back with no gap and the old screen in the new snapshot. It needs no new dependency
   and meets the Phase 3 exit test ("a remote session survives a client IP change").
2. **Then, for C13's mosh-like feel, our own authenticated UDP datagrams** in
   `rusty_mux_transport`: ChaCha20-Poly1305 from `rusty_aead`, keyed through the TLS
   connection above. The server sends to the source of the newest authenticated datagram,
   so a roam costs no handshake. The prototype roams on the next keystroke and ignores
   replayed and forged datagrams.
3. **Not QUIC.** `rustls` 0.23 ships a `quic` module (handshake and packet keys), but
   `rusty_tls` does not expose it, and the transport itself (packets, loss recovery,
   congestion, streams) would still be `quinn`: about 37k lines of third-party Rust
   (`quinn` 5.6k, `quinn-proto` 29k, `quinn-udp` 2.3k at the locked versions, in the
   graph today only through `reqwest`). Head-of-line-free streams buy little here: the
   mux sends one ordered frame stream per connection.

## Measurements

Loopback, release build, Linux; TCP rows from three runs, the Unix row from five requests. Prototype: `server/tests/roaming.rs`
and `server/tests/roaming_udp.rs` in the patch kept with the project files
(`mux/spike4/roaming-prototype.patch`).

| Path | Result |
|---|---|
| TCP dial from the new address | 1.0 ms |
| TCP Hello to Welcome (resumed) | 51 ms |
| TCP attach to Ready (snapshot in) | +50 ms |
| Unix socket, any request round trip | 50 ms, every time |
| UDP, first datagram from the new address to the reply there | 0.6 ms, no handshake |

Over a real link a TCP roam costs one RTT for TCP, one for TLS 1.3, one for Hello and
one for attach, after the client notices the old path is dead. UDP costs none beyond
the next datagram.

## Found on the way

- **50 ms per wake in `rusty_tokio`'s `LocalRuntime`.** `LocalRuntime::block_on` parks
  on the runtime's condvar, but a `spawn_local` task's wake notifies the `LocalSet`'s own
  condvar (`rusty_tokio/src/runtime/mod.rs`, the comment above `shared.park(0)`), so the
  loop sleeps out its 50 ms park timeout. strace shows the reactor thread waking one
  futex while the server thread times out on another. `rusty-mux-server` runs on
  `build_local`, and every connection is a `spawn_local` task, so **each request and
  keystroke waits up to 50 ms today, locally too.** This is the first thing to fix.
- **A reconnect is a new client.** It gets a new `ClientId`; the input lease and the
  client's label stay with the old one until the server sees the old socket close. A
  path that dies without a FIN keeps it open for minutes, so the prototype's resumed
  client was refused the lease (`LeaseHeld`). Remote mode needs a resume token that
  hands the old client's lease and label to the new connection and drops the old one,
  plus a heartbeat so a dead path is noticed in seconds.
- **`conn::run` takes a `UnixStream`.** Making it generic over a read and a write half
  is about ten lines (in the patch); the TCP and TLS listeners then share it.

## Predictive echo hook

Transport-independent, and small: the server reports, on `Output`, the `req` of the last
`Input` it had written to the PTY before reading that output (mosh's echo ack). The client
shows a prediction until an output acks its keystroke, then drops it. A feature bit and
an optional field; no server-side prediction.

## Tailnet

Over `rusty_tail`'s userspace netstack (`ts-net`), the server keeps one tailnet IP and
WireGuard follows the client's endpoint, so a TCP session there should survive an IP
change without a reconnect. Unverified: it needs two nodes and a control server, and
`rusty_tail` moving to `crates/libs/` is an owner gate (ROADMAP Phase 3).

## Proposed order (each its own PR)

1. Fix the `LocalRuntime` wake in `rusty_tokio`, with a latency test.
2. Generic `conn::run`; TCP + `rusty_tls` listener behind an explicit flag, with client
   resume tokens and a heartbeat. Test: a client changes address and keeps its lease.
3. Echo ack (`Output`) for predictive echo; the client's prediction display.
4. `rusty_mux_transport`: UDP datagrams keyed from the TLS session. Needs `rusty_tls` to
   expose `export_keying_material` (a `rusty_tls` change, an owner gate).

# Spike 4: roaming transport

Question ([SPIKES.md](SPIKES.md) row 4, [RISKS.md](RISKS.md) R3): which transport carries a
remote session across a client IP change, UDP or QUIC, and where predictive echo hooks in.
Run 2026-10-28 on `main` at 7197c347. Report only: the prototype is not for merge, and
its numbers are the author's loopback runs, not reproduced elsewhere or measured in
production. The owner approved the order below on 2026-10-28 (DECISIONS); the
`rusty_tls` exporter in step 4 stays a separate owner gate.

## Answer

1. **First, TCP + `rusty_tls`, with reconnect-and-resume.** The protocol already
   survives a reconnect: `Hello { resume }` replays missed events (`Features::EVENT_SEQ`,
   `server/src/history.rs`), and attach re-sends the block's screen as a checkpoint. The
   prototype moved a client from 127.0.0.2 to 127.0.0.3 over TCP and got the event stream
   back with no gap and the old screen in the new snapshot. That shows event replay and
   the snapshot carry a session across an address change, with no new dependency. It
   does **not** yet pass the Phase 3 exit test ("a remote session survives a client IP
   change"): the resumed client could not take back its input lease (below), and there
   is no remote listener or identity yet.
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
| Unix socket, request round trip on an otherwise idle server | 50 ms, all five |
| UDP, first datagram from the new address to the reply there | 0.6 ms, no handshake |

Over a real link a TCP roam costs one RTT for TCP, one for TLS 1.3, one for Hello and
one for attach, after the client notices the old path is dead. UDP costs none beyond
the next datagram.

## Found on the way

- **50 ms per wake in `rusty_tokio`'s `LocalRuntime`.** `LocalRuntime::block_on` parks
  on the runtime's condvar, but a `spawn_local` task's wake notifies the `LocalSet`'s own
  condvar (`rusty_tokio/src/runtime/mod.rs`, the comment above `shared.park(0)`). A
  parked loop with nothing else to do therefore sleeps up to its 50 ms park timeout
  before it runs the woken task; other runnable work or a wake of the runtime's ordinary
  queue can drive local tasks sooner. strace shows the reactor thread waking one futex
  while the server thread times out on another. `rusty-mux-server` runs on `build_local`
  and every connection is a `spawn_local` task, so **on an otherwise idle server a request
  or keystroke can wait up to 50 ms, locally too**; in the runs above every request did.
  This is the first thing to fix.
- **A reconnect is a new client.** It gets a new `ClientId`; the input lease and the
  client's label stay with the old one until the server sees the old socket close. A
  path that dies without a FIN keeps it open for minutes, so the prototype's resumed
  client was refused the lease (`LeaseHeld`). Remote mode needs a resume token that
  hands the old client's lease and label to the new connection and drops the old one,
  plus a heartbeat so a dead path is noticed in seconds.
- **`conn::run` takes a `UnixStream`.** Making it generic over a read and a write half
  is about ten lines (in the patch); the TCP and TLS listeners then share it.

## Predictive echo hook

Transport-independent, and **only a heuristic**: no ack proves that some output reflects
some input. This section records the open problem; step 3 must design exact causality,
epochs and suppression, and have them reviewed, before any implementation.

- **Ack, as mosh has it.** With a feature bit, the server stamps output with `echo_ack`:
  the newest `Input` `req` written to the PTY at least a grace period (mosh uses 50 ms)
  before that output was read. It says the echo, if there is one, has *probably* had time
  to arrive. It does not say this output is the echo.
- **Check against the screen.** Once an output carries `echo_ack >= req`, the client
  compares each prediction for that keystroke with its grid. A match confirms it; a
  mismatch drops the pending predictions and lowers confidence.
- **This can be wrong both ways.** Input written at t=0, then unrelated output read at
  t=60 ms, carries that input's ack before any echo: the check rejects a correct
  prediction (a false rejection), or a cell that happens to hold the predicted character
  confirms a wrong one (a false confirmation). A slow program's echo after the grace
  period gives the same false rejection. Resetting predictions on a reconnect or resume
  limits the damage but does not fix this.
- **Sensitive input.** A prediction draws the typed character before the program has
  echoed it, so at a password prompt it would show the password, underlined or not, for
  as long as it lasts. Step 3 must suppress predictions when the program has turned
  echo off (on Unix the PTY's termios shows it; on Windows this is open), and test it.

## Tailnet

Over `rusty_tail`'s userspace netstack (`ts-net`), the server keeps one tailnet IP and
WireGuard follows the client's endpoint, so a TCP session there should survive an IP
change without a reconnect. Unverified: it needs two nodes and a control server, and
`rusty_tail` moving to `crates/libs/` is an owner gate (ROADMAP Phase 3).

## Order (each its own PR; owner-approved 2026-10-28)

1. Fix the `LocalRuntime` wake in `rusty_tokio`, with a latency test.
2. Generic `conn::run`; TCP + `rusty_tls` listener behind an explicit flag, with client
   resume tokens and a heartbeat. Test: a client changes address and keeps its lease.
3. Predictive echo: exact causality, epochs and password suppression designed and
   reviewed first (above); then the echo ack and the client's prediction display.
4. `rusty_mux_transport`: UDP datagrams keyed from the TLS session. Needs `rusty_tls` to
   expose `export_keying_material` (a `rusty_tls` change, an owner gate).

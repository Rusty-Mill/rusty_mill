# UDP transport: design for review

Status: proposed 2026-10-30. This is Phase 3 step 4 (C13), and the owner said
Go on 2026-10-30, and answered its four open questions the same day (below).
Nothing here changes code or a wire format yet. Build PRs follow once the
design is approved.
Background: [SPIKE-4-ROAMING.md](SPIKE-4-ROAMING.md) and the step 2 decision
in [DECISIONS.md](DECISIONS.md) (SSH bootstrap, 2026-10-29).

## Goal and limits

`rusty-mux --ssh HOST` should keep working through a change of client address
(Wi-Fi to mobile, a new DHCP lease, NAT rebinding) without a handshake. The
next authenticated datagram from the new address carries on. A dead path
should cost no TCP retransmission backoff. This is the Phase 3 exit test, "a
remote session survives a client IP change".

The design does not cover the following:

- No QUIC (owner, 2026-10-28).
- No state-sync protocol in the mosh style. The mux frame stream stays as it is.
- No path MTU discovery.
- No local use: Unix sockets stay as they are.
- Windows: the client's `--ssh` is Unix-only today. The server side uses
  `rusty_tokio`'s `UdpSocket` and works on both platforms.

## The `rusty_tls` gate goes away

Spike 4 planned to key datagrams from a TLS session through the TLS exporter,
which would have been a separate owner gate. Step 2 replaced TLS with an SSH
bootstrap, so this design keys datagrams the way mosh does instead. The server
hands a fresh random key to the client over the connection SSH already
authenticated and encrypts. The design needs no TLS and no `rusty_tls` change.

## Bootstrap

An `rusty-mux --ssh` client holds several connections to the server: one for
control and one for each block it shows, and it opens another whenever it
shows a new block. Each connection is its own client on the server. UDP
therefore gives the client process one **association** (one key and one
`id`), and the association carries any number of **streams**. Each stream is
one connection, served exactly as a socket connection is. The association
takes the place of the SSH forward: anything the process could open through
the forward, it can open as a new stream, with no further SSH login.

1. The control connection connects as today, through the SSH forward. In its
   Hello it offers a new feature bit, `DATAGRAM`, alongside `TAKEOVER`.
2. A server started with `--udp ADDR:PORT` echoes `DATAGRAM` in Welcome. A
   server without that flag does not, and nothing below happens.
3. The client sends `OpenDatagram` and the server replies `Datagram { port,
   id, key }`:
   - `id` is 8 random bytes naming the association.
   - `key` is 32 bytes from `rusty_rand`, the OS CSPRNG.

   The key is never logged, never written to disk, and travels only inside
   the SSH channel. Any process that can reach the server's socket can ask
   for one. That is the same trust as the socket itself, which belongs to the
   user, and the same as every request today.
4. The client resolves the server's host itself, from the `hostname` that
   `ssh -G HOST` prints, unless the user gives `--udp-host`.
5. **Probe.** The client sends a `Probe` chunk on the association, opening no
   stream, and waits for the server's authenticated `Probe` reply. If the
   reply does not come within 2 s, the client gives up on UDP: UDP is blocked,
   or the path goes through a `ProxyJump` or a firewall. The client stays on
   the SSH forward and says so once in its status line. **Nothing has moved
   yet**, so there is nothing to undo. A late reply is ignored, and the
   client sends `Close`.
6. **Move.** Only once the probe has succeeded does each connection open a
   stream and send its `Hello` with `Takeover { client, token }`. The control
   connection goes first, then each pane. The stream then takes its client
   over (make before break, as in #1243), with attachment, lease, history
   cursor and all. The server ends the old SSH connection on takeover, as it
   does today.
7. When every connection has moved, the client stops the SSH forward. From
   then on, a newly shown block opens a new stream on the association.

**A Welcome that is lost after the probe.** A stream is reliable, so a
Welcome lost in a single datagram is simply sent again. If the path dies
during a move, the Hello may already have taken the client over while its
Welcome never arrives. The client is then in the same position as #1243 with
a dead SSH path, and recovers the same way:

- It retries over the association, which may roam.
- If the association is gone, it opens a fresh SSH connection (BatchMode) and
  takes the client back with its last confirmed token, which the server
  accepts (#1251).

The same covers a Hello delivered late, after the client has abandoned a
stream. The server takes the client over and ends its SSH connection, and the
client takes it back as above. No state exists in which the client cannot
reach its own client record.

## Datagram

```
| id: 8 | seq: 8 (top bit = direction) | ciphertext | tag: 16 |
```

- The cipher is ChaCha20-Poly1305 from `rusty_aead`. The nonce is 4 zero bytes
  followed by `seq`, and the AAD is `id` and `seq`. The top bit of `seq` is 0
  from client to server and 1 from server to client, so the two directions
  never share a nonce under the one key. `seq` is never reused. If it ever
  reaches 2^63, the association is closed.
- The maximum UDP payload is 1200 bytes. That fits the IPv6 minimum MTU with
  room for headers, so nothing is fragmented and no PMTU discovery is needed.
- The plaintext is a list of chunks:
  - `Data { stream, offset, bytes }`: a slice of one stream's bytes in this
    direction. The first `Data` for a new stream number, from the client,
    opens that stream.
  - `Ack { stream, cumulative, limit, ranges: up to 4 }`: what has arrived of
    that stream from the other side, and the highest offset the receiver will
    accept (its window).
  - `Echo { stamp }`: the newest send time seen from the peer, used to
    measure RTT.
  - `Probe`.
  - `Close { stream }`, or `Close` for the whole association.

  Each stream carries the existing encoded mux frames, unchanged, so
  everything above the transport (Hello, takeover, history replay, snapshots,
  echo acks) works as it does over a socket.

## Receiving and roaming

- Each side checks, in this order, before any state changes:
  1. **Direction.** The top bit of `seq` must be the incoming one: 0 at the
     server, 1 at the client. Both directions share one key, so without this
     check a server datagram reflected back to the server would open
     correctly.
  2. **`id`.** The server looks up the association; the client compares the
     `id` with its own.
  3. **Tag.**
  4. **Replay window.** This is a 1024-bit bitmap below the highest `seq`
     seen.

  A datagram that fails any check is dropped with no reply, and only a
  counter is kept. Only one that passes all four touches the replay window,
  the peer address, acks or stream data. An unauthenticated sender therefore
  gets nothing back, so the port cannot be used for amplification or as a
  scanner's oracle.
- The server sends to the source address of the authenticated datagram with
  the **highest** `seq` so far. A roam takes effect on the client's next
  datagram, and a late datagram from the old address cannot move the server
  back.
- What remains: an on-path attacker who copies a fresh datagram and wins the
  race from a spoofed source can redirect the server's replies until the
  client's next datagram. That is a denial of service, not a leak, since the
  attacker cannot read what is redirected. Mosh has the same property.

## Reliability

The transport is a small ARQ over each stream's ordered bytes, in each
direction. It is a pure state machine with no I/O, so it can be tested on a
simulated link. Congestion control and RTT belong to the association, while
ordering, acks and flow control belong to each stream.

- **Retransmission:** SRTT and RTTVAR follow RFC 6298, measured from `Echo`.
  The RTO stays between 100 ms and 2 s, and backoff stops at 2 s. A fast
  retransmit fires when three later chunks are acked past a gap.
- **Congestion:** a NewReno-style window in bytes. It starts at 10 datagrams,
  is halved on a loss, and never goes below 2 datagrams. Pacing is not
  included in the first version.
- **Acks:** acks ride on data. A lone `Ack` goes out within 20 ms if there is
  no data to carry it.
- **Liveness:** each side sends a datagram after 3 s of silence. The client
  treats the path as lost after 6 s with nothing heard, keeps retransmitting
  at the capped RTO, and shows the outage the way #1243 already does for a
  dead SSH path.
- **Flow control:** each stream's receive buffer holds at most 1 MiB in
  total: bytes in order but not yet read, plus bytes out of order. Every
  `Ack` sets `limit` to the cumulative offset plus the free space, and a
  sender never sends past `limit`. When a reader stalls, the window closes
  and the sender stops. The server's `serve` loop, for example, awaits a PTY
  write before reading the next request. Data already acknowledged is never
  dropped. While the window is closed, the sender probes it once per RTO.
- **Send bounds:** on the server, bytes unacked or held back by a closed
  window count against the outbox's existing 8 MiB budget. On the client, a
  write blocks once 1 MiB is queued on a stream, as a socket write blocks.
- **Association bounds:** at most 64 open streams, and at most 1024
  associations per server. An `OpenDatagram` past that limit is refused.

## Overflow and catch-up

A stream is never resynchronized in place. Its bytes either arrive complete
and in order, or the stream is closed. Catching up always happens on a new
stream, through the existing takeover, resume and attach path.

- When a stream's server-side bytes pass the 8 MiB budget, the server closes
  that stream and discards its queued bytes. This happens, for example, to a
  fast-printing block while the path is dead or the laptop sleeps. **This
  differs from a socket today.** An over-budget socket client is dropped from
  every map (`State::disconnect`), lease and all. An over-budget stream's
  client instead becomes an orphan, as a `TAKEOVER` client whose connection
  ends does now, and it is marked **stale**. It waits out its grace of
  `--udp-idle` (24 h by default; see Session lifetime), and its lease is
  released after 30 s. Sockets keep today's behaviour.
- When the client returns, it opens a new stream and takes its client over
  with its token. Its Hello carries its event cursor. If that cursor is still
  in the server's recent history, the missed events are replayed. If it is
  not, the Welcome does not echo it, and the client refetches state as a new
  client does today.
- A takeover keeps the client's attachments, so on its own it would send no
  new checkpoint. A **stale** client's takeover therefore re-sends each
  attached block's screen as a checkpoint before any new output, as attach
  does, and clears the mark. The screen is then current, whatever was
  discarded.
- Silence without overflow behaves the same way, minus the eviction: the
  stream survives, the server retransmits when the client is heard again,
  and nothing is replayed.

## Where it lives

- A new crate, `rusty_mux_transport` (the crate C13 names). It holds the pure
  association: sealing, opening, the replay window, streams, the ARQ, flow
  and congestion control, and the choice of peer address. Its interface is
  `on_datagram(now, from, bytes)`, `write(stream, bytes)`, `read(stream)`,
  `poll_transmit(now) -> Option<(addr, datagram)>` and `next_timeout()`. It
  has no clock, socket or thread of its own.
- **Server:**
  - `conn::run` becomes generic over a read half and a write half, about ten
    lines (spike 4).
  - One task owns the UDP socket and demultiplexes datagrams by `id`, then by
    stream. Each new stream gets a pair of in-memory halves and its own
    `conn::run`, so a stream is served exactly as a socket connection is.
- **Client:**
  - `Conn<S>` is already generic over a `Read + Write` stream.
  - A thread owns a std `UdpSocket` and the association, and hands out a
    blocking `Read + Write` handle for each stream. Where the client opens a
    connection today (`reach`), it opens a stream once the move is done.

## Session lifetime (decided: B)

Today a `TAKEOVER` client whose connection ends is kept for 30 s
(`ORPHAN_GRACE`), lease included, and then dropped. Over UDP nothing tells
the server that a connection has ended. Silence is the only signal, and a
laptop that sleeps for an hour is silent too. A stream's `Close`, or the
association's `Close`, ends its connections as a socket close does.

- **B, as mosh does (chosen by the owner).** The association, and its
  clients (live, or orphaned after an overflow), survive silence for up to
  24 h, set with `--udp-idle`. The **lease** is released after 30 s of
  silence, so other clients can type. On return the client carries on with
  no SSH, retakes the lease if it is free, and catches up as described under
  Overflow and catch-up. The 8 MiB budget is unchanged. The longer retention
  keeps the client record and the association, not the output. Once
  `--udp-idle` passes, the association and its clients are dropped, and the
  client must log in again over SSH, as `--ssh` reconnect already does.
- *A, not chosen:* would have treated 30 s of silence as the end of each
  connection, followed by the usual 30 s grace and then a new SSH login.

## Tests (when built)

- **Transport, pure:** a seeded simulated link with loss, reordering,
  duplication and delay must deliver every stream intact and in order, in
  both directions. The tests also check:
  - a stalled reader in either direction closes the window, the sender's
    memory stays bounded, and nothing acknowledged is lost;
  - a forged tag, a wrong `id`, a replayed `seq` and a datagram reflected
    back to its sender (wrong direction) are each dropped without a reply,
    and leave the replay window, peer address and stream state unchanged;
  - the reply address follows the highest `seq`;
  - the RTO and window respond to loss.
- **Server and client, end to end on loopback:**
  - A client on 127.0.0.2 roams to 127.0.0.3 mid-session and keeps its
    attachment and lease, with no reconnect (the Phase 3 exit test).
  - With UDP blocked, the client falls back to the SSH stream within 2 s.
  - When the path dies after the probe, whether in one direction only,
    mid-move, or with the Welcome lost, the client ends with its own client
    record over UDP or a fresh SSH connection. A late Hello does the same.
  - After SSH has stopped, a split shows a new block on a new stream.
  - A block that prints more than 8 MiB while the path is dead: its stream is
    evicted, the client stays, and on return the client resumes on a new
    stream with the current screen.
  - A captured trace contains no typed plaintext.
  - Silence past 30 s releases the lease but keeps the client; silence past
    `--udp-idle` (shortened in the test) ends the association and its
    clients.
- **Over a real local sshd,** as in #1236 and #1239: bootstrap, SSH stops
  after UDP is up, and a roam.

## Decided by the owner

Nano agreed with all four proposed answers on 2026-10-30 19:28Z:

1. **Session lifetime: B.** An association and its clients survive silence
   for up to 24 h by default (`--udp-idle`). A client's lease is released
   after 30 s of silence.
2. **Opt-in, one port.** The server listens for UDP only with `--udp
   ADDR:PORT`, using one port for every association. The default port is
   60001.
3. **Client default: automatic.** `--ssh` uses UDP when the server offers
   `DATAGRAM` and falls back to SSH after 2 s. `RUSTY_MUX_UDP=off` keeps it on
   SSH.
4. **Dependencies.** The mux crates may depend on the first-party crates
   `rusty_aead` and `rusty_rand`; the lockfile changes by those lines only.

## Build order, once approved

One PR each:

1. `rusty_mux_transport`: the pure association and its simulated-link tests.
2. Server: `DATAGRAM`, `OpenDatagram`/`Datagram`, `--udp`, the socket task,
   streams, and the generic `conn::run`.
3. Client: probe, moving every connection, fallback, stopping SSH, new
   streams for new blocks, outage display, and the end-to-end roam test.
4. ROADMAP and CAPABILITIES (C13 done), and the manual check over a real sshd.

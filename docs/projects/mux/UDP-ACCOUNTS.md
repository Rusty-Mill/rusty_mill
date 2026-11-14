# UDP for account servers behind the login front door (card #1461): design note

Status: Proposed. Nano gave the go ("UDP", 2026-11-14T02:52Z, message
`cmsg_014f6o52E2iAAAgMm9v8hh1wLaiMgwjmNyUVjci1wgFdjp`). He has not confirmed the design below
on the PR, and Dottius's security review is still owed (SHARED-MODE.md child table). Nothing
is built. Pack rows reserved: D268 (the go), D269 (defaults below), Q66.

## Context

Login mode (C26) relays each connection from a root front door to a per-account server.
`main.rs` refuses `--udp` on the front door and on an account server, so an account's client
stays on the SSH path and has no roaming (C13). The UDP transport needs no wire change for
this: the server's `Datagram { port, id, key }` reply already names the port, and the client
resolves the host itself (UDP-TRANSPORT.md, Bootstrap 3-4).

## Design

1. **No wire change, feature bit, crate or dependency.** An account server accepts `--udp`
   when forwarded; the offer, the association and its principal work as on a personal server
   (an association belongs to its opener's principal, `datagram.rs`).
2. **The front door never touches UDP.** It is root; it parses no datagrams and opens no UDP
   socket. `--login MAP --udp-ports ADDR:LO-HI` (off by default) only passes `--udp
   ADDR:LO-HI` to the account servers it starts.
3. **The account server picks its own port.** `--udp` takes a single port or a range
   `LO-HI`; the server binds the first free port in it (as itself, so `LO` must be 1024 or
   more) and the `Datagram` reply names the one it got. The kernel arbitrates, so there is no
   port table in the front door, a front door restart loses nothing, and an account server
   that restarts simply binds again. No free port: the server starts without UDP and its
   clients stay on SSH, as today.
4. **Tailnet principals get no UDP.** They come through the same preface, so the account
   server refuses `OpenDatagram` for a `ts:` or `ts-node:` principal (the C27 rule: a tailnet
   guest has no UDP).
5. **Shared mode, the tailnet and TLS listeners stay without UDP.** Only login-mode accounts.
6. **Windows unchanged:** UDP is Unix-only (C79).
7. **A hard hour.** An account server started with `--udp` by the front door gets `--udp-idle
   3600` and `--udp-lifetime 3600`: every association ends an hour after it began, however
   much it is heard (an idle bound alone would never fire for a client the 3 s keepalive keeps
   talking). The client then asks again, through the front door and the map (see Risks, 1).
   `--udp-lifetime SECS` is a new flag of `rusty-mux-server` and is off unless given.
8. **Optional UDP.** On an account server, any failure to bind (range taken, an address not
   on the host yet) is logged once and the server serves without UDP. `--udp-ports` takes only
   `ADDR:LO-HI` with `LO` 1024 or more; IPv6 needs its brackets.

## Risks

1. **Admission is checked once.** After an association is up, its streams skip the front
   door: the map, a SIGHUP reload, PAM (D266) and the per-uid caps. A principal removed from
   the map keeps its UDP clients until the association's hard end, one hour after it began. Built
   so after review: an idle bound alone never fired for a client that kept talking. Later
   option: drop associations on a reload, which needs the front door to tell the account
   server.
2. **Squatting.** Any local user can bind a port in the range first. That denies UDP to an
   account (it stays on SSH) and nothing more: the client learns the port from its own
   authenticated `Datagram` reply and every datagram is sealed under the association's key.
3. **Posture.** `ADDR` is what the host exposes; it is explicit, never a default, and a range
   of N ports is N accounts at most with UDP. Past that, clients fall back to SSH and the
   client says so once.
4. **Guessing.** An association has a 32-byte key and an 8-byte id; an unauthenticated
   datagram gets no reply, so no amplification (UDP-TRANSPORT.md). Account servers are
   separate processes with separate sockets, so one account cannot reach another's.
5. **Tests and docs.** The `--udp` refusals in `main.rs` tests, its doc comment and USAGE
   change; SHARED-MODE.md and CAPABILITIES C26 lose "UDP for account servers".

## Tests when built

A login-mode test with a fake account: an account server with `--udp 127.0.0.1:LO-HI` binds
the first free port and a second account gets the next; a full range starts without UDP; a
roam keeps its client; a tailnet principal's `OpenDatagram` is refused; parse tests for the
range and for the refusals that remain.

## Questions for Nano

- Q66: is one hour the right idle bound for account servers, or drop associations on a map
  reload? (Recommend 1 h now, reload drop later: a gap of an hour for a removed user.)

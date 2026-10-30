---
category: Added
changelog: rusty-mux-server --udp ADDR[:PORT] offers clients a UDP association, and serves each of its streams as a connection.
---
## 2026-10-30 - rusty_mux server listens for UDP

- **Added:** build step 2 of `docs/projects/mux/UDP-TRANSPORT.md` (Phase 3 step 4, C13), the server side.
  - `rusty-mux-server --udp ADDR[:PORT]` listens for UDP on one port, 60001 unless given, for every association. Without the flag nothing changes.
  - A client that offers the new feature `DATAGRAM` may send `OpenDatagram`. The reply, `Datagram { port, id, key }`, travels only inside its connection. The id and key come from `rusty_rand`; the key is never logged. An offer that sees no datagram within 60 s is forgotten.
  - The first authentic datagram under an offered id makes the association live. Each stream the client opens on it is served by the same connection code as a Unix socket, so a stream can say Hello, take a client over, attach and type.
  - `--udp-idle SECS` (a day unless given) ends an association that has been silent that long, and with it its streams. Their clients then go as a closed socket's do.
- **Changed:** `conn::run` takes any read half and write half, not only a Unix socket's. `rusty_mux_server` and `rusty_mux_tui` declare `rust-version` 1.98.1, the floor of `rusty_aead` beneath `rusty_mux_transport`.
- **Known limitations:** no client uses this yet; that is build step 3. Two parts of the owner's lifetime decision B come in their own step: releasing a silent client's lease after 30 s, and keeping a stream's client as a stale orphan when the stream overflows its 8 MiB budget, then re-sending its screens on takeover. Until then an over-budget stream's client is dropped as a socket's is.

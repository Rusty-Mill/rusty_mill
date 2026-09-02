# Changelog

## v0.1.0

First release: a Rust port of [croc](https://github.com/schollz/croc)
(schollz/croc, v10), **wire-compatible with the stock Go implementation** —
rusty-croc sends to, receives from, relays for, and reconnects alongside
stock croc.

### Features

- Secure peer-to-peer file/folder transfer over a PAKE-authenticated relay.
- Send files, folders, text (`--text`), stdin (`-`), and zipped folders (`--zip`).
- Local-network path: auto-started local relay, IPv4/IPv6 multicast discovery,
  the `ips?` probe hand-off, and direct `--ip` connections.
- Reconnect-and-resume after a mid-transfer relay drop (ReconnectVersion 1).
- Resume/skip via `xxhash` / `imohash` / `highway` / `md5` file hashing.
- Upload throttling (`--throttle`), `.gitignore` mode (`--git`), path
  exclusion (`--exclude`), QR codes (`--qr`), config persistence (`--remember`).
- SOCKS5 / HTTP-CONNECT proxies (`--socks5` / `--connect`); public-DNS relay
  resolution (`--internal-dns`).
- A full relay server (`relay`) that stock croc clients can use.

### Security

- Constant-time PAKE curve arithmetic for **every** curve: p256/p384/p521 via
  RustCrypto, and croc's nonstandard SIEC via a from-scratch Montgomery backend.
- Key material zeroized on drop.
- `cargo-fuzz` harnesses for the frame, message, PAKE, and mnemonicode parsers.

### Quality & performance

- Verified byte-for-byte and end-to-end against the real Go binary in both
  directions (`scripts/interop_test.sh`, 8 transfer groups).
- 49 unit tests, including Go-generated crypto/curve/wire vectors.
- CI runs fmt + clippy + tests + the live interop suite on every push/PR.
- Faster throughput than Go croc at equal memory and ~⅓ the binary size;
  near-parity handshake latency (`scripts/bench.sh`).

See [MIGRATION.md](MIGRATION.md) for the full architecture analysis, module
mapping, and the constant-time / scalability write-ups.

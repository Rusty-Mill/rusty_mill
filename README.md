# rusty-croc 🦀🐊

A Rust port of [croc](https://github.com/schollz/croc) — a tool for easily and
securely transferring files and folders between two computers, using a code
phrase and (when needed) a relay.

**Status: phase 3 — feature parity for everyday use.** The protocol layers,
relay server, file-transfer engine, and the local-network path are ported and
**wire-compatible with stock croc v10**: rusty-croc can send to stock croc,
receive from stock croc, and relay for stock croc — files, folders, text, and
zipped folders, with resume/skip, local-relay hand-off, direct `--ip`
connections, throttling, and all four hash algorithms (verified end-to-end;
see `scripts/interop_test.sh`). See [MIGRATION.md](MIGRATION.md) for the full
analysis, module mapping, and roadmap.

## Usage

```sh
# Send a file or folder (prints a code phrase)
rusty-croc send my-file.txt

# Receive on the other machine (rusty-croc or stock croc, either works)
rusty-croc receive 1234-foo-bar-baz
CROC_SECRET=1234-foo-bar-baz croc     # stock croc receiving from rusty-croc

# Start a relay that stock croc clients can use
rusty-croc relay --ports 9009,9010,9011,9012,9013 --pass pass123

# Health-check any croc relay (Go or Rust)
rusty-croc ping croc.schollz.com:9009
```

```sh
# More ways to send
rusty-croc send --text "quick message"      # send text instead of a file
echo hi | rusty-croc send -                 # send stdin
rusty-croc send --zip my-folder             # zip the folder first
rusty-croc send --throttle 10M big-file     # limit upload speed

# Receive directly from a sender on your network (no relay round-trip)
rusty-croc receive --ip 192.168.1.5:9009 <code>
```

Useful flags: `--relay host:port`, `--pass`, `--yes` (skip the accept
prompt), `--overwrite`, `--curve p256|p384|p521|siec`,
`--hash xxhash|imohash|highway|md5`, `--no-compress`, `--no-multi`,
`--local` (LAN only), `--no-local`.

When sender and receiver share a network, rusty-croc behaves like croc: the
sender starts a local relay and announces it (UDP multicast + the `ips?`
side-channel through the relay), and the receiver hops onto it so data never
leaves the LAN.

## What's ported

| Area | Status |
|---|---|
| Frame protocol (`comm`) | ✅ byte-compatible |
| Encryption (`crypt`: PBKDF2+AES-GCM, Argon2id+XChaCha20) | ✅ vector-tested against Go |
| Message envelope (`message`, JSON+DEFLATE+AES) | ✅ vector-tested against Go |
| PAKE (schollz/pake v3, incl. the SIEC curve) | ✅ live-tested against Go, both roles |
| Code phrases (`mnemonicode`) | ✅ vector-tested against Go |
| Relay server + relay client handshake (`tcp`) | ✅ end-to-end with stock croc |
| File-transfer engine (`croc`: send/receive, folders, resume) | ✅ interop-tested both directions |
| Local-network path (discovery, local relay, `ips?` probe, `--ip`) | ✅ interop-tested both directions |
| Text sending, stdin, `--zip`, `--throttle`, imohash/highway | ✅ interop-tested |
| Reconnect-after-drop, proxies, `--remember`, QR codes | ⬜ phase 4 |

## Development

```sh
cargo test                  # self-contained tests incl. Go-generated vectors
scripts/interop_test.sh     # live Go↔Rust interop (needs Go toolchain + git)
```

The module layout mirrors the Go source tree (`src/comm.rs` ↔ `src/comm`,
etc.) to keep the port reviewable side by side with upstream.

## License

MIT, matching upstream croc. The mnemonic word list is from the original
mnemonicode project (Oren Tirosh, MIT).

# rusty-croc 🦀🐊

A Rust port of [croc](https://github.com/schollz/croc) — a tool for easily and
securely transferring files and folders between two computers, using a code
phrase and (when needed) a relay.

**Status: phase 2 — transfers work, cross-implementation.** The protocol
layers, the relay server, and the client file-transfer engine are ported and
**wire-compatible with stock croc v10**: rusty-croc can send to stock croc,
receive from stock croc, and relay for stock croc — files and folders, with
resume/skip semantics (verified end-to-end; see `scripts/interop_test.sh`).
See [MIGRATION.md](MIGRATION.md) for the full analysis, module mapping, and
roadmap.

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

Useful flags: `--relay host:port`, `--pass`, `--yes` (skip the accept
prompt), `--overwrite`, `--curve p256|p384|p521|siec`, `--no-compress`,
`--no-multi`.

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
| Local-network discovery, reconnect, zip, throttling, proxies | ⬜ phase 3 |

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

# rusty-croc 🦀🐊

A Rust port of [croc](https://github.com/schollz/croc) — a tool for easily and
securely transferring files and folders between two computers, using a code
phrase and (when needed) a relay.

**Status: phase 1 — foundations + relay.** The protocol building blocks and
the relay server are ported and **wire-compatible with stock croc v10
clients**: a plain `croc send` / `croc <code>` pair can transfer files through
a `rusty-croc relay` today (verified end-to-end; see `scripts/interop_test.sh`).
The client-side file-transfer engine (`send`/`receive`) is the next phase —
see [MIGRATION.md](MIGRATION.md) for the full analysis, module mapping, and
roadmap.

## Usage

```sh
# Start a relay that stock croc clients can use
rusty-croc relay --ports 9009,9010,9011,9012,9013 --pass pass123

# Health-check any croc relay (Go or Rust)
rusty-croc ping croc.schollz.com:9009
```

Point stock croc at it:

```sh
croc --relay myhost:9009 send some-file
croc --relay myhost:9009 <code-phrase>
```

## What's ported

| Area | Status |
|---|---|
| Frame protocol (`comm`) | ✅ byte-compatible |
| Encryption (`crypt`: PBKDF2+AES-GCM, Argon2id+XChaCha20) | ✅ vector-tested against Go |
| Message envelope (`message`, JSON+DEFLATE+AES) | ✅ vector-tested against Go |
| PAKE (schollz/pake v3, incl. the SIEC curve) | ✅ live-tested against Go, both roles |
| Code phrases (`mnemonicode`) | ✅ vector-tested against Go |
| Relay server + relay client handshake (`tcp`) | ✅ end-to-end with stock croc |
| File-transfer engine (`send` / `receive`) | ⬜ phase 2 |

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

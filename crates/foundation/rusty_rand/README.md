# rusty_rand

OS-backed cryptographically secure random bytes with **no external
third-party dependencies**: `getrandom(2)` on Linux x86_64/aarch64 (blocks until the
kernel pool is seeded), `/dev/urandom` on other Unix (handle opened once, no lock),
`BCryptGenRandom` on Windows (hand-declared FFI to `bcrypt.dll`).

```rust
let mut key = [0u8; 32];
rusty_rand::fill(&mut key)?;
let nonce = rusty_rand::bytes(16)?;
```

Extracted from three identical copies that `rusty_oauth`, `rusty_uuid`,
and `sessionmgr-proc` each carried. Errors are returned, never masked by a
fallback to a weaker source.

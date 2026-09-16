# rusty_rand

OS-backed cryptographically secure random bytes with **no external
dependencies**: `/dev/urandom` on Unix (handle opened once and cached),
`BCryptGenRandom` on Windows (hand-declared FFI to `bcrypt.dll`).

```rust
let mut key = [0u8; 32];
rusty_rand::fill(&mut key)?;
let nonce = rusty_rand::bytes(16)?;
```

Extracted from three identical copies that `rusty_oauth`, `rusty_uuid`,
and `sessionmgr-proc` each carried. Errors are returned, never masked by a
fallback to a weaker source.

Not the raw `getrandom(2)` syscall: `rusty_libc` and `rustils`'
`platform-linux` cover that on Linux only, and this crate has to serve
macOS/BSD consumers through the same path.

# rusty_crypto_key

[![CI](https://github.com/baileyrd/rusty_crypto_key/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_crypto_key/actions/workflows/ci.yml)

A zeroize-on-drop key storage micro-crate for Rust.

`rusty_crypto_key` provides `SecretBytes`, a secure key container that volatile-zeroes memory on `Drop`, redacts debug prints, and performs constant-time equality comparisons. Behind the default `std` feature, `save_to_file`/`load_from_file` persist a secret to disk, restricted to `0600` (owner read/write only) on Unix; Windows has no equivalent ACL restriction applied yet.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

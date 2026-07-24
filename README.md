# rusty_wire

[![CI](https://github.com/baileyrd/rusty_wire/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_wire/actions/workflows/ci.yml)

A minimal, **zero-dependency by default** endian-explicit byte cursor `Reader` and `Writer` for Rust.

`rusty_wire` provides bounds-checked, explicit-endianness primitive integer and slice operations over byte buffers without panicking, `#![no_std]` + `alloc` by default.

## Features

- `#![no_std]` + `alloc` support by default (`std` feature enabled by default).
- Bounds-checked forward-only byte slice `Reader`.
- Growable byte `Writer` with length back-patching (`patch_u16_be`, `patch_u32_be`).
- Explicit endianness at every call site (`u16`, `u32`, `u64` in `be` or `le`).

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.
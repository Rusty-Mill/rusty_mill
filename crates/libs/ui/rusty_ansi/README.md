# rusty_ansi

[![CI](https://github.com/baileyrd/rusty_ansi/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_ansi/actions/workflows/ci.yml)

A zero-allocation, `#![no_std]` VT100 / CSI / OSC ANSI escape sequence parser core for Rust.

`rusty_ansi` parses text streams into explicit ANSI tokens (`AnsiToken::Text`, `AnsiToken::Csi`, `AnsiToken::Osc`), strips ANSI color codes, and calculates visible display width (`visible_width`).

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

# rusty_der (rusty_ansder)

[![CI](https://github.com/baileyrd/rusty_ansder/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_ansder/actions/workflows/ci.yml)

A minimal, **zero-dependency by default** ASN.1 BER/DER TLV encoder and decoder built on [`rusty_wire`](https://github.com/baileyrd/rusty_wire).

`rusty_der` provides bounds-checked, explicit-tag ASN.1 TLV reading and writing (`INTEGER`, `OCTET STRING`, `BOOLEAN`, `SEQUENCE`) without panicking, `#![no_std]` + `alloc` by default.

## Features

- Built directly on top of `rusty_wire` cursors (`Reader` / `Writer`).
- Safe, bounds-checked ASN.1 DER length & TLV tag parsing.
- Definite short and long length forms (ITU-T X.690).

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.
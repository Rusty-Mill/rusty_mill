# rusty_compress

[![CI](https://github.com/baileyrd/rusty_compress/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_compress/actions/workflows/ci.yml)

A sans-IO stream compression and decompression abstraction crate. It currently
implements only raw stored-block (non-compressed) RFC 1951 DEFLATE; the
`CompressionLevel` parameter has no effect, and Gzip/Zlib wrapper support is
not yet implemented.

`rusty_compress` provides clean, safe `compress_deflate` and `decompress_deflate` primitives.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

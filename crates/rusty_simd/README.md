# rusty_simd

[![CI](https://github.com/baileyrd/rusty_simd/actions/workflows/ci.yml/badge.svg)](https://github.com/baileyrd/rusty_simd/actions/workflows/ci.yml)

A zero-dependency SIMD (AVX2/NEON/FMA) accelerated block dequantization kernel library for LLM and Whisper inference in Rust.

`rusty_simd` provides `dequantize_q4_0`, `f16_to_f32`, and SIMD dot product kernels for `rusty_llama` and `rusty_whisper`.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

# Vendored A2A specification

`a2a.proto` in this directory is copied, unmodified, from the canonical
[A2A protocol specification repository](https://github.com/a2aproject/A2A)
(`specification/a2a.proto`), at commit
[`19598c4`](https://github.com/a2aproject/A2A/commit/19598c4baddbbaf868595cf9f3119c89ec96329f).
It is licensed under Apache-2.0 by the A2A Protocol Working Group; see
`LICENSE` at the repository root.

It is included here purely as a reference for anyone verifying this
crate's [`rusty_a2a::types`] against the normative source, or wanting to
generate a gRPC binding on top of [`rusty_a2a::types`] with `tonic-build`.
This crate's own code does not compile or otherwise depend on this file -
`rusty_a2a::types` and, for the gRPC binding, `rusty_a2a::pb` are hand-written,
field-for-field Rust transliterations of it, not codegen output. `rusty_prost/tools/gen_vectors.py`
reads it (with the `googleapis/` files it imports) to make the wire vectors that `pb` is
tested against.

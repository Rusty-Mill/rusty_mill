# Vendored googleapis annotations

`google/api/{annotations,http,client,field_behavior,launch_stage}.proto`
are copied, unmodified, from the
[googleapis](https://github.com/googleapis/googleapis) repository
(`google/api/`), licensed under Apache-2.0 by Google LLC.

`specification/a2a.proto` (vendored one directory up, at `../a2a.proto`)
imports `annotations.proto`, `client.proto`, and `field_behavior.proto`
directly, and transitively needs `http.proto` and `launch_stage.proto`
that those import. `protoc` cannot resolve `import` statements without
the imported files present on its include path, and these five aren't
bundled with `protoc` itself (unlike the `google/protobuf/*.proto`
well-known types, which are) - so building `a2a.proto` at all, for any
purpose (gRPC codegen, `--descriptor_set_out`, ...), requires vendoring
them somewhere. This directory is that somewhere; `build.rs` points
`tonic-build` at it with `-I`.

Only what's actually needed is included - not the rest of googleapis.

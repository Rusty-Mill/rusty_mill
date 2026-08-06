fn main() {
    #[cfg(feature = "grpc")]
    generate_grpc_code();
}

/// Compiles `spec/a2a.proto` (the vendored, normative A2A protocol
/// definition) into the `A2AService` server trait and message types used
/// by `src/server/grpc.rs`. Requires a `protoc` binary on `PATH` (see
/// spec/googleapis/README.md for why the vendored `google/api/*.proto`
/// files under `spec/googleapis/` are also needed as an include path).
#[cfg(feature = "grpc")]
fn generate_grpc_code() {
    println!("cargo:rerun-if-changed=spec/a2a.proto");
    println!("cargo:rerun-if-changed=spec/googleapis");

    // Client codegen is enabled too (`pb::a2a_service_client`) even though
    // `rusty_a2a::client::A2aClient` only speaks JSON-RPC: it's used by
    // this crate's own gRPC integration test, and left public as a bonus
    // for anyone who wants a raw tonic client instead.
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["spec/a2a.proto"], &["spec", "spec/googleapis"])
        .unwrap_or_else(|e| panic!("failed to compile spec/a2a.proto: {e}"));
}

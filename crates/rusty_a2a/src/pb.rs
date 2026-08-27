//! Generated `tonic`/`prost` types for the A2A gRPC binding (spec Section
//! 10), compiled from the vendored `spec/a2a.proto` by `build.rs` (via
//! `tonic-prost-build`; requires a `protoc` binary on `PATH` to build this
//! crate with the `grpc` feature at all).
//!
//! Lives at the crate root - rather than nested under [`crate::server`] -
//! so that it's reachable by both the gRPC server
//! ([`crate::server::grpc`]) and the gRPC client ([`crate::client::grpc`])
//! with just the `grpc` feature, independent of `server`/`client`.
#![allow(clippy::doc_lazy_continuation)]

tonic::include_proto!("lf.a2a.v1");

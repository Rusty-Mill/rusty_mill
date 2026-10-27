---
category: Changed
changelog: "`rusty_a2a` is marked `publish = false`: it depends on unpublished workspace crates and its gRPC server API changed in #1022."
---
## 2026-10-27 - rusty_a2a: publish = false

- **Changed:** `rusty_a2a/Cargo.toml` sets `publish = false`. The crate already could not be published: its optional `rusty_ip` and `rusty_tonic` dependencies are themselves `publish = false`. Marking it makes that explicit, so a `cargo publish` fails with a clear message instead of a missing-dependency error, and records that `grpc_router(handle)` and `serve_grpc` (changed in #1022) are not a stable outside API.
- **Known limitations:** `adk-a2a` depends on `rusty_a2a` and has no `publish` key, so it is unpublishable for the same reason; it is left as it is here.

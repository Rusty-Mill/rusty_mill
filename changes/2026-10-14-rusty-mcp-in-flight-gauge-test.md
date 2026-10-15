---
category: Added
changelog: Add a rusty-mcp test that reads the mcp.server.requests.in_flight gauge back through the OpenTelemetry SDK's in-memory exporter.
---
## 2026-10-14 - rusty-mcp in-flight gauge test

- **Added:** `tests/otel_in_flight.rs` reads `mcp.server.requests.in_flight`
  back through the SDK's in-memory metric exporter and expects 1 while a request
  is open and 0 after `request_finished`. Previously, changing `.add(-1, ...)`
  to `.add(1, ...)` left the suite green. Test-only: `opentelemetry_sdk`'s
  `testing` feature is a dev-dependency feature, `Cargo.lock` is unchanged, and
  production code is untouched.
- **Known limitations:** covers the direct `Instruments` lifecycle only, not
  request cancellation or per-label balancing.

---
category: Changed
changelog: `rusty-mcp`'s metrics label decisions (the cardinality guard) now live in a backend-free core (`otel::labels`); `Instruments` and `McpMetricsLayer` are thin adapters over it. `Outcome` and `TaskOutcome` keep their `otel::metrics` paths.
---
## 2026-10-14 - rusty-mcp: sans-IO metrics core (sovereignty item 5, metrics part)

- **Why:** which methods and names may become labels, how a status becomes an outcome, and which attributes each instrument carries are the cardinality guard, and they were mixed with `opentelemetry` calls and `tower`. They now stand alone, so the OTLP exporter (item 9) can replace the OpenTelemetry crates without touching them.
- **Added:** crate-private `otel::labels`: `method_label`, `KnownNames::name_label`, `header` (with the `rmcp` base64 wrapper), `Outcome::from_status`, and the attribute sets per instrument (`in_flight_labels`, `duration_labels`, `request_labels`, `task_labels`). It imports no `opentelemetry`, `tower`, `axum` or `tokio`.
- **Changed:** `Instruments` and `McpMetricsLayer` keep their public API and behaviour: they turn the core's labels into `KeyValue`s and record. `Outcome` and `TaskOutcome` moved to the core and are re-exported from `otel::metrics`, so their public paths are unchanged. The label unit tests moved beside the core, with new ones for each instrument's attribute set.
- **Not changed:** instrument names, units, buckets, label keys and values, and the wire output. Nothing is public that was not before.
- **Verified:** `rusty-mcp` lib tests and the `otel_metrics` and `otel_export` suites pass with all features. Each of these mutations failed a test: no method guard, `resources/read` named, no name allow-list, the outcome on the histogram, 403 as a client error, the counter losing its outcome, the abandoned count recorded as failed.
- **Known limitations:** no test notices if the in-flight gauge stops being decremented (found while mutating; it was already untested, because the OTLP tests scan bytes and cannot read a gauge value). Catching it needs the SDK's in-memory exporter. The core sits under the `otel` feature, so its tests need that feature. This completes the `limits`, `auth` and metrics cores of item 5.

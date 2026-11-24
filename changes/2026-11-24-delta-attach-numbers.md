---
category: Changed
changelog: Incremental attach deltas stay on paper; DELTA.md now records measured reconnect costs and a do-not-build recommendation.
---
## 2026-11-24 - Incremental attach deltas: measured reconnect cost (#83)

- **Changed:** `docs/projects/mux/DELTA.md` gains measured RTCP v6 size and encode/decode times (403 KB, about 2 ms and 5-7 ms for 200x50 + 10k history), computed link times, and a recommendation not to build deltas. No code or wire change.
- **Known limitations:** link times are computed, not measured over SSH or UDP; the workload is synthetic. Nano's call is not recorded yet.

---
category: Added
changelog: rusty_term: `bench/floors.py` checks the #889 wall-clock throughput floors on the 4 MB workloads; both are met.
---
## 2026-10-24 - rusty_term throughput floors check

- **Added:** `crates/libs/ui/rusty_term/bench/floors.py` runs `bench_vt_throughput` 3 times on the 4 MB ascii_throughput and scroll_heavy workloads, judges the slowest run against the #889 floors (58 and 16 MB/s), and prints peak RSS. On main both pass (61-110 and 34-48 MB/s on a shared host), against 14.8 and 4.6 MB/s before the ring buffer.
- **Known limitations:** manual only, since wall-clock time on a shared CI runner is noise; the instruction-count gate stays the CI check. Peak RSS needs `os.wait4` (Linux and macOS). The 200 KB CI workloads fill the scrollback late or not at all, mostly measure fresh-grid allocation, and are not judged.

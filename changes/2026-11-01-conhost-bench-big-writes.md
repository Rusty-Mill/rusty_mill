---
category: Fixed
changelog: rusty_conhost serves writes over 64 KiB a slice at a time instead of refusing them, and gains the Phase B throughput bench against ConPTY.
---
## 2026-11-01 - Own console host takes large writes; throughput bench against ConPTY

- **Fixed:** a single `WriteFile` or `WriteConsole` over 64 KiB, such as one big buffer written at once or Rust's stdout, failed on our console host with "The parameter is incorrect" (os error 87), where ConPTY accepts it. The host now reads such a write 64 KiB at a time, each slice once the last is emitted, so it still holds at most one slice per call. The call stays whole: no other call is taken until it completes, and a character split across slices arrives intact. Other APIs keep the 64 KiB per-packet limit (design §6.1, §7.4).
- **Added:** `rusty_conhost`'s `conhost_bench` example, the Phase B throughput gate (design §9.2). It writes each `gen_workloads.py` file to ConPTY and to our host, times spawn to EOF, and fails when our host is more than 10% slower on any workload. On Beast (Windows build 26300, 4 MB workloads), our host took 0.17x ConPTY's time on geomean, and 0.65x at worst (cursor_thrash).
- **Known limitations:** the bench measures throughput only, not that both hosts drew the same screen. Our host runs in the bench's process, while ConPTY runs as a separate process that repaints frames, so the ratio reflects architecture as much as parser speed.

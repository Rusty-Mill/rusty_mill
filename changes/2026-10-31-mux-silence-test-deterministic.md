---
category: Fixed
changelog: The rusty_mux UDP test for going stale mid-silence no longer races the shell's throughput against the silence window, so it passes under full-workspace CI load.
---
## 2026-10-31 - Deterministic rusty_mux stale-mid-silence test

- **Fixed:** `going_stale_mid_silence_keeps_the_silence_deadline` (rusty-mux-server) failed 3/3 under a full-workspace sweep: it needed a shell to print 8 MiB within a 1.5 s silence. It now makes the client stale directly through a test-only hook, with no await between the peer's last datagram and going stale, and waits for the silence to release the lease instead of asserting a fixed 500 ms margin. A stale lease time of 60 s still tells a restarted clock apart from the silence deadline.
- **Known limitations:** the overflow path itself is still covered only by the tests that print past the budget, which use the 20 s patience.

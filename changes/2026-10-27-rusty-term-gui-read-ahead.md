---
category: Changed
changelog: The rusty_term window reads PTY output on its own thread while the previous batch is parsed, so `cat` of styled or Unicode output finishes up to 39% faster.
---
## 2026-10-27 - GUI: read PTY output ahead of the parser

A profile of GUI `cat` throughput (Xvfb, 4 cores, CPU renderer) found that painting is not the limit: a build that never repaints is no faster. The limit is the kernel PTY plus the window's reader thread, which read and parsed in turn. The kernel holds only a few KiB, so the child stalled for every batch being parsed.

Each pane now has a `pty-read-<id>` thread that reads batches into a bounded queue of 16 (at most 1 MiB), and the existing reader thread parses from it. Parse order, replies to the child, wakeups and synchronized output are unchanged. Medians on 32 MB workloads, main → this change: `sgr_churn` 795 → 586 ms, `unicode_heavy` 920 → 565 ms, `ascii_throughput` 561 → 529 ms, `scroll_heavy` 1217 → 1180 ms (launch-to-exit wall time, so throughput, not frame latency). The last two were already near the cost of a reader that only drains the PTY.

- **Known limitations:** the screen can trail the child by up to the queued 1 MiB. The TUI front end is unchanged. Not measured on a GPU host, with `--gpu`, or under a real compositor.

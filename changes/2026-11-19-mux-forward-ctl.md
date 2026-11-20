---
category: Added
changelog: rusty-mux-ctl forward tunnels a port of the server's host to this machine.
---
## 2026-11-19 - rusty_mux: `forward` (C81, slice 2)

- **Added:** `rusty-mux-ctl forward [--block B] [LOCAL:]REMOTE...` attaches to the block, takes its input lease and listens on 127.0.0.1 for each port, tunnelling connections to the same port on the server's host over the mux connection. It prints `LOCAL<TAB>REMOTE` (JSON with `--json`) per port and runs until the block closes. It fails while another client holds the lease. The tunnel is `rusty_mux_client::forward`, std only, for the GUI to share.
- **Known limitations:** no half-close, loopback IPv4 only, no sharing between two `forward` processes.
- **Hardened:** the client takes no more than 16 forwards at once (a connection past that is closed before it costs a thread), refuses server data past 16 KiB a chunk or 256 KiB unacknowledged, and refuses an acknowledgment of more than it sent, closing that forward with a reason. A write to a slow local reader gives up when the forward or the run ends, so stop and block close always return.

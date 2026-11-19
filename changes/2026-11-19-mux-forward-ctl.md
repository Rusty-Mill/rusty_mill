---
category: Added
changelog: rusty-mux-ctl forward tunnels a port of the server's host to this machine.
---
## 2026-11-19 - rusty_mux: `forward` (C81, slice 2)

- **Added:** `rusty-mux-ctl forward [--block B] [LOCAL:]REMOTE...` attaches to the block, takes its input lease and listens on 127.0.0.1 for each port, tunnelling connections to the same port on the server's host over the mux connection. It prints `LOCAL<TAB>REMOTE` (JSON with `--json`) per port and runs until the block closes. It fails while another client holds the lease. The tunnel is `rusty_mux_client::forward`, std only, for the GUI to share.
- **Known limitations:** no half-close, loopback IPv4 only, no sharing between two `forward` processes.

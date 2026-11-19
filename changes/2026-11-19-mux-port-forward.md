---
category: Added
changelog: rusty_mux can tunnel a loopback port of the server's host over the mux connection, for the holder of a block's input lease.
---
## 2026-11-19 - rusty_mux: port forwards, the tunnel (C81, slice 1)

- **Added:** `Features::PORT_FORWARD` (bit 40) and `OpenForward` (0x27), `ForwardData` (0x2b), `ForwardAck` (0x2c) and `CloseForward` (0x2d). The server dials `127.0.0.1:port` for the holder of the block's input lease, moves bytes both ways with 256 KiB of credit a direction, and closes the forward with its connection, its block, the opener's place in the block, or either end. At most 16 forwards a connection and 64 a server. The client side (`rusty-mux-ctl forward`, the link click) follows.
- **Known limitations:** no half-close (a target that closes its write side ends the forward); no `::1`; every platform that serves can forward, but nothing in a client opens one yet.

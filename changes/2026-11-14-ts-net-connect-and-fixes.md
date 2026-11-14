---
category: Added
changelog: ts-net gains `Node::connect` (outbound TCP on the tailnet) and two fixes: big messages no longer hang, and magicsock no longer ping-loops.
---
## 2026-11-14 - ts-net connect, chunk drop and ping loop fixes

- **Added:** `ts_net::Node::connect(addr)` opens an outbound TCP stream to a tailnet peer (15 s timeout, ephemeral ports from 49152). Tests: connect carries data, refused connect is an error.
- **Fixed:** ts-net dropped a chunk when the app queue was full, so a message over about 33 KB hung. The chunk is now held until the queue drains. Test: a burst larger than the queue arrives whole.
- **Fixed:** ts-magicsock answered every disco ping with a ping, so two nodes pinged each other forever and spun a core. It now pings back only an unknown source. Test: a known endpoint is not pinged back.
- Found by spikes A and dial on Beast (pack D257); no mux crate uses `connect` yet.

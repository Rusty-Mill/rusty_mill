---
category: Added
changelog: Nodes follow a mesh hub to its standby: they learn the standby addresses from the hub and reconnect control and the relay there when the hub goes away.
---
## 2026-11-16 - Mesh hub failover, part 2: nodes reconnect (slice H8)

- **Added:** `ts-daemon hub --advertise-standby URL` (repeatable; also on `rusty-mux-server --hub`; `https://` or loopback only, like `--standby`, because nodes send their pre-auth key there). The hub puts the URLs in every netmap (`MapResponse.StandbyURLs`), so a node learns where the promoted hub will answer. A standby serves control and the relay on that one URL.
- **Changed:** the engine no longer stops when its DERP connection closes. It retries the URL it was given, then each advertised standby, backing off from 0.5 s to 5 s, and swaps the new relay connection in (magicsock too). The relay is found again in the background, so direct paths, commands and netmaps keep flowing. The netmap poll does the same, but asks for the netmap first and registers with the pre-auth key only if the hub refuses (a promoted hub that holds the copy already knows the machine, and the key may have expired or been rotated out). It refuses a control server whose key differs from the one first seen. If every handle to the engine is gone, a lost relay ends the engine as before.
- **Public API (additive):** `MapResponse::standby_urls`, `MagicSock::set_derp`, `Hub::advertise_standbys`. Engine behaviour change: DERP loss now means reconnect, not stop.
- **Tests:** two real nodes stream 2 MiB through the real `ts-daemon` hub; the active is killed mid-stream, the standby is promoted on the advertised address, and the stream finishes byte for byte; with the pre-auth key rotated out before the takeover, a new node then joins the promoted hub and an old node reaches it; a different hub on the standby address never sees the node; a dropped node stops retrying a dead hub. Also: the netmap carries the list; try order and backoff.
- **Limits:** a node only learns standbys while a hub is up, so a node that starts while the active is down has no fallback. Nodes behind `--tls-ca` trust the same anchors for the standby.

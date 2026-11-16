---
category: Added
changelog: A mesh hub can have a standby: it copies the active hub's state and key over a secret-protected link, and an operator promotes it by restart.
---
## 2026-11-16 - Mesh hub failover, part 1: replication, standby, promote (slice H8)

- **Added:** `GET /replicate[?after=N]` on the hub, off unless `--replica-secret FILE` is given (16+ characters, file mode 600, constant-time compare, answers 404 when off and 401 for a wrong secret). It sends the node table and the hub's Noise key, waits while the standby already has the latest version, and answers at least every 25 s. The hub refuses `--replica-secret` over plain HTTP unless `--listen` is a loopback address.
- **Added:** `ts-daemon hub --standby URL --replica-secret FILE [--tls-ca PEM] [--state-dir DIR]` (and the same flags on `rusty-mux-server --hub`) follows the active hub and serves nobody. The copy lands in the state dir as the hub identity and `nodes.json`, each written whole (private temporary file, then rename). `URL` must be `https://` or loopback.
- **Added:** `--listen ADDR --promote URL [--force]` serves from that copy as the new active hub. It refuses when the state dir holds no copy (it would start a hub with a new key) or while the hub at `URL` still answers `GET /key`, unless `--force`. Promotion is never automatic.
- **Public API (additive):** `Hub::enable_replication`, `NodeStore::export` (default: refuses), `ts_hub::replica::{fetch, answers, apply, follow, Snapshot, ReplicaError}`, `ts_dial::split`. `ts-derp`'s URL parsing now calls `ts_dial::split`.
- **Tests:** a promoted standby presents the same Noise key, still has every node, still refuses a machine revoked before the kill, and takes a new join, in the library and through the real `ts-daemon` binary (kill, promote, join; promote refused while the active runs).
- **Limits:** a change reaches the standby within about a second, so a revoke made just before the active dies can be lost: revoke again after promoting. The DERP relay key is not replicated (nodes do not pin it). Existing relayed sessions drop with the active and need the standby address list, which is part 2 (netmap field and client fallback).

---
category: Fixed
changelog: Mesh hub: a node the hub already holds can register again with the same machine and node key and no pre-auth key, so a restarted stock tailscaled stays logged in.
---
## 2026-11-16 - Mesh hub: re-register without a pre-auth key

- **Fixed:** the hub refused every registration without a pre-auth key. A stock `tailscaled` that registers again (after a restart) sends the same machine and node key and no key, and showed "unknown or empty pre-auth key" until `tailscale up --auth-key` was run again. The store now accepts that request when the machine is registered, not revoked, not expired, and the node key is unchanged. Address, login and expiry do not change, so it never extends a node's life.
- **Behaviour change:** a registration with no key can now succeed. It still needs the machine's Noise key and the current node key, and an expired, revoked or rotated-key node must present a key as before.
- **Found by:** running stock `tailscaled` 1.104.7 against `ts-daemon hub` (two nodes, DERP ping, 5 MiB transfer, then a hub restart).

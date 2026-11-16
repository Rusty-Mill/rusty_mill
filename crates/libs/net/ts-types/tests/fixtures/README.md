# Golden fixtures

Captured 2026-07-09 from official **tailscaled 1.86.2** (userspace
networking) joined to a local **Headscale 0.26.1** tailnet with two nodes —
the environment brought up by `interop/up.sh`.

| File | Source |
|------|--------|
| `status.json` | `GET /localapi/v0/status` on node1 (node2 active via direct path) |
| `ping.json` | `POST /localapi/v0/ping?ip=100.64.0.2&type=disco` on node1 |
| `prefs.json` | `GET /localapi/v0/prefs` on node1 |

Recapture with, e.g.:

```sh
curl -sS --unix-socket interop/state/ts1/tailscaled.sock \
  http://local-tailscaled.sock/localapi/v0/status
```

Note: `prefs.json` contains only zeroed/redacted-by-tailscaled key material
(`privkey:000…`); LocalAPI never returns real private keys.

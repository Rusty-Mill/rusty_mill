---
category: Security
changelog: rusty_mux takes a client over only for its own user, and reports UDP roams as connection changes.
---
## 2026-11-01 - rusty_mux: takeover checks the principal, roams report a connection change

- **Security:** a takeover now continues a client only when the new connection's verified principal matches the client's. A valid token presented by another principal starts a separate client and takes no attachment or lease. A personal server lets in only its owner, so nothing changes there today; this readies login mode (C26).
- **Changed:** a UDP association now belongs to the principal of the client that asked for it, and its streams carry that principal, instead of one server-wide owner.
- **Added:** when a UDP association roams to a new address, `client-connection-changed` (transport `udp`) fires once for each client it serves. No new feature bit, event kind or field.

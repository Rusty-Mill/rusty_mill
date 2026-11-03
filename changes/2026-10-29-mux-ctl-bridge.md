---
category: Added
changelog: rusty-mux-ctl bridge joins stdin and stdout to the server's socket, so a client can reach a remote server with `ssh host rusty-mux-ctl bridge`.
---
## 2026-10-29 - rusty_mux: `rusty-mux-ctl bridge`

- **Added:** `rusty-mux-ctl [--socket S] bridge` copies stdin to the server's socket and the socket to stdout, flushing after every read, until the server closes the connection. The end of stdin is passed on as the end of the client's writes. The server checks the bridge's own credentials as for any local client, so over SSH the user's login is the identity. An unreachable socket exits 4. Phase 3 step 2b: the fallback for servers that refuse the socket forward `rusty-mux --ssh` uses; DECISIONS 2026-10-29.
- **Known limitations:** the `rusty-mux` client does not use it yet; choosing it when the forward is refused comes with the reconnect work.

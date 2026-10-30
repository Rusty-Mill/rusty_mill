---
category: Added
changelog: rusty-mux --ssh moves onto UDP when the server offers it, keeps working through a change of address, and falls back to SSH within 2 s.
---
## 2026-10-30 - rusty-mux --ssh moves onto UDP

- **Added:** build step 3 of `docs/projects/mux/UDP-TRANSPORT.md` (Phase 3 step 4, C13), the client side.
  - The control connection offers `DATAGRAM`. When the server echoes it, the client asks for an association over the SSH forward, sends to the `hostname` that `ssh -G HOST` prints (or `--udp-host HOST`), and probes it.
  - Once the probe is answered within 2 s, the control connection takes its client over on a stream of the association, and the SSH forward stops. Each block shown later opens its own stream.
  - The association is served on a private local Unix socket, as the forward was, so every connection, takeover and reconnect works as before. It keeps working when the client's address changes.
  - With no answer within 2 s, the client stays on SSH and says so once. `RUSTY_MUX_UDP=off` skips UDP. If the move itself fails, the client takes its client back over SSH.
  - A lost path first probes the association again; if it does not answer, the client logs in over SSH (BatchMode) as before.
- **Known limitations:** after falling back to SSH from a lost association, the client does not move back to UDP. The 6 s outage display, the lease release after 30 s of silence and the stale orphan on overflow come in their own step. A real sshd check is build step 4.

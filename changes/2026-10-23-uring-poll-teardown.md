---
category: Fixed
changelog: Release io-uring readiness polls before closing registered sockets so listeners can immediately rebind.
---
## 2026-10-23 - Complete io-uring poll teardown before socket close

- **Fixed:** `rusty_tokio`'s optional io-uring reactor now consumes the terminal poll and cancellation completions and synchronizes with kernel request cleanup before deregistration returns. This releases kernel socket references before listener close, fixing intermittent `AddrInUse` when a server immediately restarts on the same address.
- **Fixed:** Submission queue pressure cannot discard poll removal, and registration identities prevent stale completions from targeting reused fd numbers. Cancellation drives its own ring progress; ring waits and readiness callbacks do not hold the driver mutex.
- **Testing:** Real-kernel regressions cover a stalled reactor callback, deregistration inside a callback, and cancellation with a full submission queue while retaining unrelated readiness completions. Existing tonic reconnect tests are unchanged.
- **Known limitations:** The optional backend serializes ring access and synchronously waits for kernel cancellation during deregistration; io-uring must be permitted by the host. Fatal ring errors are surfaced rather than treated as completed teardown. The default epoll backend is unchanged.

---
category: Fixed
changelog: rusty_tokio `broadcast::Receiver::recv` is now cancel safe: a dropped or re-polled `recv()` no longer leaves wakers queued until the next send.
---
## 2026-10-31 - rusty_tokio: a cancelled broadcast recv leaves no waker

- **Fixed:** every pending poll of `broadcast::Receiver::recv` pushed a new waker clone, and nothing removed it when the future was dropped, so `loop { select! { m = rx.recv() => .., _ = tick.tick() => .. } }` on an idle channel grew without bound until the next `send`. Each pending `recv()` now keeps one keyed waker slot, replaces its waker in place on a re-poll and removes the slot when it resolves or is dropped, as tokio's `Recv` does. Displaced and removed wakers are dropped after the channel lock is released. The public signature of `recv` is unchanged. Four new tests in `sync::broadcast` fail on the base and pass now (#577).
- **Known limitations:** removing a slot is a linear scan of the channel's parked receivers, which is fine for the few receivers a channel has in this workspace.

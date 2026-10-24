---
category: Added
changelog: rusty_orch grants hosted goals a permission lease of paths and an end, and the BBP host confines every pool member's sandbox to it.
---
## 2026-10-24 - rusty_orch: permission leases for hosted goals

- **Added:** `orch_core::factory::Lease` on `Grant`: absolute, normalized read and write paths and an optional end. A leased parent delegates or re-grants only leases it covers, and `Factory::restore` re-checks it for bound children. `Fence` carries the lease of its generation.
- **Added:** the hosting configuration's optional `lease: {read, write, ttl_secs}`. Each `run` takes the assignment over with that lease and a fresh end, durably, before any host command. The BBP host freezes the task's pool with each assigned member's sandbox met with the lease, both resolved through symlinks, keeping nothing beneath a kept write root. An unsandboxed assigned member, or a checkout outside the lease, is refused. A later fence with a lease that reaches other paths is refused, so widening access needs a new task; an ended lease is refused as unavailable, and each moderator's `--max-wall-secs` is cut to the lease's end so BBP stops its harnesses then. Cancel is never refused for a lease. New acceptance mode `lease-confined`. ADR-0017.
- **Changed:** assignment records are schema v2 in `assignments-v2.mmap`. A goal saved only as v1 loads as unleased, and its next save writes v2; the v1 file is never written again.
- **Known limitations:** no network flag, since BBP has no per-harness network switch to enforce one. A renewal with an earlier end does not shorten a moderator already running. BBP still adds system roots, the turn workspace and, with `checkout`, the repository's object store. Maintenance cycles host unleased. Fair admission (3d) is a separate slice.

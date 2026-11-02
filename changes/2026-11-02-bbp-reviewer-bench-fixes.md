---
category: Fixed
changelog: A permission lease now bounds the validator profile's read roots, cancel no longer fails for a leased goal whose pool the lease cannot confine, and a BBP session server exits when its client closes stdin during a wait.
---
## 2026-11-02 - BBP leases bound profile roots; leased cancel; session wait sees stdin close

- **Fixed:** under a lease, candidate code could read every `read_roots` entry of the validator profile set, which the lease never narrowed (#1325). orch-bbp now refuses at freeze a profile root, resolved through symlinks, that is neither a system root nor within the lease, or that lies beneath a confined member's write root, and freezes each root as the path it resolved to, so a link a harness re-points later cannot move it. ADR-0017 records the rule.
- **Fixed:** cancelling a leased goal whose task was never frozen failed when a pool member was unsandboxed, because cancel froze the task confined (#1331). Cancel now freezes such a task unleased, opens it straight into cancelled and starts nothing; a later leased holder is refused.
- **Fixed:** a `bbp mcp` session server blocked in `wait` ignored its client closing stdin until the wait's timeout, up to 600 s (#1329). On Linux the wait now polls the request stream for a hang-up and ends within one recheck interval of it, and the server exits; requests already sent are still answered.
- **Known limitations:** the hang-up check is Linux-only and needs a pipe or socket for input. A leased profile whose toolchain lives outside the system roots (a rustup sysroot, say) now needs that path in the lease's read set.

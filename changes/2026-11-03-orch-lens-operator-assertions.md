---
category: Added
changelog: rusty_orch reads the lens operator's no-overage and host-cleared assertions from a file only the operator can change, with a 30-day lapse and a machine binding.
---
## 2026-11-03 - rusty_orch: lens operator assertions file

- **Added:** `rusty_orch::lens::assertions` (Let Em Loose #594, Nano's conditions of 2026-11-03). The operator lists `no_overage` entries (provider, account, date checked) and `host_cleared` entries (provider, machine id, date checked) in a JSON file outside any repository. A no-overage check lapses after 30 days; a host-cleared entry holds only on the machine whose `/etc/machine-id` it names. Unknown fields, missing accounts, bad or future dates are refused with the field named.
- **Added:** `load` refuses the file when the runner's user could change it or any directory above it (it owns it, it is root, or it is group- or other-writable outside a sticky directory), so an agent running as that user cannot assert anything. No file means no assertions. Non-Unix hosts assert nothing.
- **Changed:** `Registry::build` takes the per-provider `Grants` instead of one `Assertions` for every provider.
- **Known limitations:** the ownership check protects the operator only when the runner runs as a different user from the file's owner. Nothing reads the file yet; the `rusty_orch lens` subcommand comes next.

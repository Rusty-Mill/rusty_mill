---
category: Added
changelog: rusty_orch reads the lens operator's no-overage and host-cleared assertions from a file only the operator can change, with a 30-day lapse and a machine binding.
---
## 2026-11-03 - rusty_orch: lens operator assertions file

- **Added:** `rusty_orch::lens::assertions` (Let Em Loose #594, Nano's conditions of 2026-11-03). The operator lists `no_overage` entries (provider, account, date checked) and `host_cleared` entries (provider, machine id, date checked) in a JSON file outside any repository. A no-overage check lapses after 30 days and names its account: the Claude adapter admits a question only when `claude auth status` reports that same account (`email`), and a Codex entry, since Codex cannot report its account, must also name the dedicated machine and holds only there; a host-cleared entry holds only on the machine whose `/etc/machine-id` it names. Unknown fields, missing accounts, a provider named twice in one list, bad or future dates are refused with the field named.
- **Added:** `load` refuses the file when the runner's user could change it or any directory above it (it owns it, it is root, or it is group- or other-writable outside a sticky directory), so an agent running as that user cannot assert anything. No file means no assertions. Non-Unix hosts assert nothing.
- **Changed:** `Registry::build` takes the per-provider `Grants` instead of one `Assertions` for every provider. `ClaudeAgent::lens_no_overage` takes the asserted account; a login to another account, or one that does not report its account, is `Unavailable` and the reported account never reaches the error.
- **Known limitations:** the ownership check protects the operator only when the runner runs as a different user from the file's owner. Nothing reads the file yet; the `rusty_orch lens` subcommand comes next.

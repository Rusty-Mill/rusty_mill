---
category: Added
changelog: Mill Board admins can revoke every registered agent unused for over 24 hours, with a dry-run count first (settings_registered_agent_revoke_stale and a Settings button).
---
## 2026-11-17 - Mill Board revokes stale registered agents

- **Added:** `settings_registered_agent_revoke_stale` (admin) revokes registered agents last used, or created if never used, more than `older_than_hours` ago (2 to 720, default 24). It is a dry run that returns the count and ids unless `revoke: true`, and writes the registry once. Settings > Agents has a "Revoke stale" button that shows the count and asks before revoking. This frees a connector that has hit its 64-agent cap without revoking each agent by hand.
- **Fixed:** a registered agent's last use is now written back to the registry file at most an hour late (it was never written for an agent used continuously, so a restart could show it as idle for as long as it had existed). The button revokes only the agents in the previewed list that are still stale.
- **Known limitations:** it runs only when an admin asks; there is no schedule. The registry file can lag an agent's real last use, so after a restart an agent not used since counts as last used at the restart: nothing is stale until the chosen age has passed since the board started (up to 24 hours at the default; 2 hours at the minimum), and a stale-revoke right after a deploy finds nothing. If the registry file cannot be written, a revoke still takes effect in memory and reports that it must be retried.

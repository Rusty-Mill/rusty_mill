---
category: Added
changelog: Mill Board admins can rename roles from Settings > Roles; agents holding the role move with it.
---
## 2026-11-01 - Mill Board: manage roles fully from Settings (card #448)

- **Added:** `settings_role_rename` (admin only) and a Rename button on Settings > Roles. Every agent holding
  the role moves to the new name and keeps its token. It runs in three writes (add the new name, move the
  agents, drop the old name), each leaving the files valid. If the agents cannot be moved, the added name is
  rolled back; whatever fails, any leftover name is held by no agent and the error says to remove it.
- **Changed:** the Roles tab shows which agents hold each role, disables Remove for a role still held, and
  asks before removing. Creating roles, choosing their capabilities (with each capability's description)
  and the keep-an-admin guard already existed.
- **Known limitations:** capabilities stay code-defined; a new capability still needs code.

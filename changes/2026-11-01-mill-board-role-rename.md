---
category: Added
changelog: Mill Board admins can rename roles from Settings > Roles; agents holding the role move with it.
---
## 2026-11-01 - Mill Board: manage roles fully from Settings (card #448)

- **Added:** `settings_role_rename` (admin only) and a Rename button on Settings > Roles. Every agent holding
  the role moves to the new name and keeps its token. It runs in three writes (add the new name, move the
  agents, drop the old name), each leaving the files loadable whatever a failed write left behind. Nothing is
  rolled back: a failure keeps both names and the error says how to finish. A roles change is also refused if
  the saved credentials file would no longer load with it, since a failed save can still have replaced it.
- **Fixed:** with a per-model token the page no longer sends actor and role, so renaming or changing your own
  role no longer strands the page until a reload.
- **Changed:** the Roles tab shows which agents hold each role, disables Remove for a role still held, and
  asks before removing. Creating roles, choosing their capabilities (with each capability's description)
  and the keep-an-admin guard already existed.
- **Known limitations:** capabilities stay code-defined; a new capability still needs code.

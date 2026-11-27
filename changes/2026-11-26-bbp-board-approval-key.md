---
category: Security
changelog: BBP now trusts the production Mill Board's approval key (#447), so owned plan and merge approvals with a board-signed receipt verify instead of being refused.
---
## 2026-11-26 - BBP trusts the board's approval key (#447)

- **Changed:** `crates/apps/rusty_bbp_host/keys/board-approval.ed25519.pub` holds the public key the production Mill Board logged when it first started with `MILL_BOARD_APPROVAL_KEY` (D194). Until this file was filled in, every owned approval was refused. `bbp` builds after this change accept receipts the board signs with that key, and `bbp metrics` vouches for them.
- **Known limitations:** if the board's key file is lost or replaced, this file needs a new reviewed PR, and receipts signed with the old key stop verifying. Orch's gate relay and the board inbox still don't pass receipts.

---
category: Security
changelog: Mill Board's board_number_cards needs an authenticated admin credential; a caller-named admin role is refused (#1439).
---
## 2026-11-12 - Mill Board: board_number_cards needs a credential

- **Security:** `board_number_cards` now refuses a caller-named actor and role, as `inbox_list` and `inbox_act` already did. Only a token or OAuth credential holding admin, unlimited to projects, may run it (card #1439).
- **Known limitations:** on a board started without per-model credentials (stdio, `--auth-token`, or open), the settings ops and gated `card_move`s still trust a caller-named role holding admin. A board run with `--identities` is not affected.

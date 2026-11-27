---
category: Added
changelog: New read-only `bbp claims` host verb and Mill Board `lel_claims` and `lel_spend` ops for the Let Em Loose tab (#1416).
---
## 2026-11-27 - bbp claims and the LEL read ops

- **Added:** `bbp claims --dir D --task T` prints every claim of a claims task as a JSON array of claim views (read-only). Mill Board `lel_claims` returns it for a task a card links and `lel_spend` returns `[{id, spent, budget}]`; both need an authenticated admin credential. Approved by Nano (D220).
- **Deploy:** the root-owned `mill-board-bbp-guard` must admit `claims` (one word, see `1092-deploy-layout.md`) and `bbp` must be the new build; until then `lel_claims` answers "verb refused".
- **Known limitations:** brief text is not served (D221), so there is no `lel_intent` yet.

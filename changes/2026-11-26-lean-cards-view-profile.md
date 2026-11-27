---
category: Added
changelog: Mill Board credentials can carry an agent view profile that pages board_list at 50 and trims card_get to recent comments.
---
## 2026-11-26 - Lean cards: per-credential view profile (Mill Board #1437)

- **Added:** an optional `"view": "agent" | "full"` on a `credentials.json` entry (default `full`; the loader still refuses unknown keys and values). Under `agent`, a `board_list` call with neither `limit` nor `cursor` is paged at 50 and carries `next_cursor`, and `card_get` defaults to `history: "recent"` (last 5 comments, the latest handoff, every open blocker or question, `comments_omitted`).
- **Compatible:** `full`, and every credential without the key, behaves exactly as before. The profile changes defaults only: never what a credential may see or do, and an explicit `limit`, `cursor` or `history` always wins. `card_take` and `card_claim` still return the whole card. A minted worker (`agent_enrol`) keeps its lead's profile.
- **Known limitations:** the profile is set by editing the credentials file; there is no Settings screen for it yet. Nothing is switched on by this change: a credential opts in by carrying the key.

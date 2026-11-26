---
category: Added
changelog: Mill Board comments gain a kind, an id and a resolves link, and card_get can return only the recent comments.
---
## 2026-11-26 - Lean cards: typed comments and recent history (Mill Board #1436)

- **Added:** an optional `kind` on `card_comment`: `note` (default), `claim`, `handoff`, `blocker`, `question` or `decision`. A handoff is at most 3 non-empty lines and 400 bytes (done, next, where the details are); other kinds keep only the 2 KiB cap.
- **Added:** every new comment gets an `id`. A later comment can name it in `resolves` to close a `blocker` or `question`; a comment that is not an open blocker or question on that card is refused, and a resolved one is never reopened. Nothing is inferred from prose.
- **Added:** `card_get` takes `history: "recent"`: the last 5 comments, the latest handoff and every open blocker or question, with `comments_omitted`. The default stays `all`, so existing callers see no change; the agent view profile (#1437) will make `recent` its default.
- **Compatible:** comments written before this change have no kind, id or resolves and read as notes; nothing is rewritten.
- **Known limitations:** the legacy rusty_tick store keeps plain notes only: a comment with a kind other than `note`, or with `resolves`, is refused there with a clear message instead of being saved as a note. The web UI does not yet show the kind.

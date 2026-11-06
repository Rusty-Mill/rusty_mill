---
category: Added
changelog: Mill Board cards have a separate prompt field, set with card_update and shown in the card dialog with a Copy prompt button; stored in the card's board document.
---
## 2026-11-05 - Mill Board: a prompt field on cards

- **Added:** `Card.prompt`, instructions for whoever takes a card, kept apart from the body. `card_update` takes `prompt` (`""` clears it, at most 32 KiB), and `card_get` and `board_list` return it. The card dialog shows it in its own box with a Copy prompt button. Card #745.
- **Changed:** `Board::edit_at` takes a `domain::Edit` (title, body, tags, prompt) in place of separate arguments.
- **Storage:** the prompt lives in the card's rusty_tick `board` document and is left out while empty, so existing documents are unchanged and there is no migration. The tick store checks the whole document against rusty_tick's 64 KiB limit before writing, so an edit it would refuse changes nothing.
- **Partial saves:** with rusty_tick, an edit of the prompt together with title, body or tags is two fenced writes (task, then board document), as `card_plan` with priority and links already was. If the second fails (a lost connection, or another writer in between), the board answers `409` with `saved_rev` and a message saying what was saved, never a plain refusal (`BoardError::PartlySaved`). The card dialog then keeps the unsaved prompt, says only the prompt is missing, and Save finishes it on the saved revision. If another writer came in between (the card is past `saved_rev`, or its text or prompt changed), the usual Keep mine / Take theirs banner shows instead.
- **Known limitations:** `card_add` doesn't take a prompt yet; set it with `card_update`. Existing prompt sections in card bodies are not moved out.

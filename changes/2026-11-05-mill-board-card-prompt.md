---
category: Added
changelog: Mill Board cards have a separate prompt field, set with card_update and shown in the card dialog with a Copy prompt button; stored in the card's board document.
---
## 2026-11-05 - Mill Board: a prompt field on cards

- **Added:** `Card.prompt`, instructions for whoever takes a card, kept apart from the body. `card_update` takes `prompt` (`""` clears it, at most 32 KiB), and `card_get` and `board_list` return it. The card dialog shows it in its own box with a Copy prompt button. Card #745.
- **Changed:** `Board::edit_at` takes a `domain::Edit` (title, body, tags, prompt) in place of separate arguments.
- **Storage:** the prompt lives in the card's rusty_tick `board` document and is left out while empty, so existing documents are unchanged and there is no migration. The tick store checks the whole document against rusty_tick's 64 KiB limit before writing, so an edit it would refuse changes nothing. rusty_tick's own UI doesn't show it (Nano's choice, 2026-11-05).
- **Known limitations:** an edit of the prompt together with title, body or tags is two fenced requests to rusty_tick, as `card_plan` already is; a dropped connection or another writer between them leaves the task fields applied and reports an error, and saving again completes it. `card_add` doesn't take a prompt yet; set it with `card_update`. Existing prompt sections in card bodies are not moved out.

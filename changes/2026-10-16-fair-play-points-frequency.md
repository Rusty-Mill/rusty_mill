---
category: Added
changelog: rusty_fair_play cards take points (Fibonacci, with a suggestion from the card's text) and a frequency, and the deck filters by frequency.
---
## 2026-10-16 - Fair Play: points and frequency on cards

- **Added:** a card carries `points` (1, 2, 3, 5 or 8) and a `frequency` (Daily to As needed), set in the detail pane and shown on the tile. Until points are assigned the card shows `suggestedPoints`, computed from the words in its Conception, Planning and Execution plus its standards of care. Likewise `suggestedFrequency` is the period its text names most often ("daily", "each week", "seasonal"…), else As needed. The deck gains a frequency filter (by the assigned frequency, else the suggested one), and the Balance page shows each person's points and their cards by frequency.
- **Changed:** both marks live in a new sidecar, `card-marks.json`, so no stored record or data directory needs migrating; they are part of the card's etag.
- **Known limitations:** the suggestion is a heuristic tuned to the shipped deck, not a measure of effort, and the filter is client-side. Balance points and frequency counts cover leaf cards only, and count the suggestions where nothing is assigned. Only about a third of the shipped deck names a period, so the rest recommend As needed.

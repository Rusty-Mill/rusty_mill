---
category: Added
changelog: Mill Board cards can name a parent card and a milestone, with progress rolled up and a trace view (cards #766, #771).
---
## 2026-11-06 - Mill Board parent and milestone links

- **Added:** a card can name one parent, the card it was split from, so arc 2 › S2 › S2a is recorded rather than implied
  by titles. `card_add` and `card_plan` take `parent`; a loop is refused. The new `card_trace` returns the chain up to
  the root and the tree below, each node with its children's progress. A parent whose children are all Done is
  flagged ready to close and never closes itself.
- **Added:** a card can link to one milestone of its project, separate from its parent. `card_plan` takes `milestone`,
  `board_milestones` returns each milestone's progress, and the roadmap shows it beside the milestone.
- **Added:** the card dialog shows the chain and the children, with parent and milestone fields; a parent's ticket
  shows its pieces count.
- **Known limitations:** both links live in the board's document in rusty_tick, so rusty_tick's own UI does not show
  them. Moving a card to another project keeps its milestone link. The `milestone:<slug>` tags on PythRust #561,
  #759 and #396 are converted by hand after deploy.

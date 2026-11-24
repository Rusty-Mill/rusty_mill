---
category: Fixed
changelog: Mill Board states the movement rules in force in board_columns and board_whoami, so agents stop reporting moves as impossible without trying them.
---
## 2026-11-24 - Mill Board states which moves a caller may make

- **Fixed:** agents told Nano they could not move a card out of Backlog or to Done. The board refused neither: its movement rules are off, but `board_columns` listed only the transition table, which reads as a restriction (and lists no Backlog to Done path although a move with merged evidence works). `board_columns` and `board_whoami` now carry `movement` (`enforced` or `open`) and a plain-words note; the server instructions say to try `card_move` and report the refusal reason verbatim, and that a rule from a document is a rule, not a board limit. The runner ignores the two new fields.
- **Known limitations:** the transition list itself is unchanged; roles and gates are untouched.

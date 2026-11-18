---
category: Changed
changelog: The Let Em Loose grounding bundle also audits claims the board dropped, not only escalated ones (Q59, D158).
---
## 2026-11-18 - Let Em Loose grounding bundle covers dropped claims

- **Changed:** `docs/projects/let-em-loose/1166-grounding.sh` now writes a section for every claim that is Escalated or Dropped (it reads each claim's board history with `bbp why`). A dropped claim shows the rule that fired, every scoring vote with its note and citations, dormant votes posted after the drop, the scores for all votes, the latest and the first vote per author, and a mechanical hint: the score of the votes that carry no flag. New flag: a vote citing another vote. Nano's Q59 answer: a drop on ungrounded votes decides whether a question has a survivor, as in smoke-5 claim 7. The audit itself stays Claude's and it still acts on nothing.
- **Known limitations:** tested on a throwaway `bbp` store with a claim shaped like smoke-5's claim 7 (dropped at -10, mechanical hint -4 from one counter author) and one shaped like smoke-4's claim 6; not run on BaileyAi.

---
category: Fixed
changelog: **BBP verdict binding covers replacement turns.** A turn above one live turn now starts with everything others appended in the current state unread, so a judge that reads nothing, or a replacement turn after an expiry or abort, cannot verdict over a finding appended before its grant.
---
## 2026-10-15 - BBP verdict binding: the read base is the stage entry

- **Fixed:** the verdict refusal (#720) tracked only content appended during a turn. A replacement turn (after a deadline or an abort) started with nothing unread, so a judge refused for an unread finding could wait out its deadline and be granted a turn that verdicts freely. `Turn.unseen` is now seeded at grant with every message and artifact another principal appended since the state was entered that the role may read. Derived in the fold; no log change; one live turn unchanged.
- **Read base, stated:** the entry into the stage (`entered_rev`). Content from earlier stages is covered by the candidate, run and spec bindings, not by this.
- **Tests:** a judge that reads nothing after an abort is refused and then accepted after reading; the terminal path (refused retries, deadline expiry, regrant through `grant`, the same refusal, escalation on the turns budget, no turn minted by a refusal, fold equals state). Both fail with the seeding removed. The property test and the N=1 goldens are unchanged and pass.
- **Cost:** a judge must read the stage's new content before it can verdict, including after consulting a human.

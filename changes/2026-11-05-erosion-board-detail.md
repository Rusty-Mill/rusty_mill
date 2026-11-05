---
category: Added
changelog: The Mill Board Decisions view shows the erosion gate's numbers on an escalated claim: shares, limits and worst functions when erosion fails it, and unmeasured code always.
---
## 2026-11-05 - Erosion gate: detail on the Mill Board

- **Added:** Mill Board's Decisions view renders `bbp inbox`'s `erosion` field (card #704 slice 3, D91). When erosion fails the claim, it shows concentration and duplication as base → candidate percents with the delta and the enforced limits, plus each worst-affected function's complexity change. On every escalated claim it shows `unmeasured` code (D45), and a bin error as "Erosion could not measure". `inbox_list` passes the field through as the host wrote it.
- **Tests:** a unit test that the field passes through, and `tests/ui/erosion.mjs`, a browser check against `fake_bbp.sh`'s new erosion mode.
- **Known limitations:** shares and worst functions on claims erosion doesn't fail are in the JSON but not rendered, until OPEN_QUESTIONS Q23 is answered.

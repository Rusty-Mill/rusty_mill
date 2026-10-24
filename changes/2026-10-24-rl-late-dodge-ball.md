---
category: Fixed
changelog: The late_dodge and prompt_dodge tape scenarios place the ball where the game left it, so late_dodge's ball error drops from 4242.7 uu to 0.1 uu.
---
## 2026-10-24 - late_dodge ball placement

- **Fixed:** `late_dodge` and `prompt_dodge` were the only tape scenarios with no ball. The game left the ball at kickoff centre, (0, 0, 93), but a scenario with no ball puts it at (3000, 3000, 93.15), so rb-verify scored a constant 4242.6 uu ball error (3000·√2) on `late_dodge` from tick 0. Both scenarios now put the ball at (0, 0, 93.15). `late_dodge` scores ball mean 0.1 / max 2.4 uu, and the golden-capture gate now checks its ball too (bound 0.2 / 3.1).
- **Known limitations:** physics is unchanged; the car error is the same.

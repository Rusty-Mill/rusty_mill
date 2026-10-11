---
category: Changed
changelog: `rb_chaser_bot` ran on the real game for a short check (both teams chased; orange's attack direction and long runs not established).
---
## 2026-10-11 - rb_chaser_bot: first real-game check

- **Verified on the game:** Windows 11, RLBot v5, Freeplay map. The bot printed the right car and team for blue (car 0, team 0) and orange (car 1, team 1), ran with no connection error, and exited cleanly at match end; the blue car was seen driving at the ball, boosting and chasing it, and the orange car was seen chasing and driving the ball into a goal.
- **Docs:** the crate README notes that RLBot v5 joins `root_dir` onto the manifest's folder, so it must stay relative.
- **Known limitations:** Freeplay assigns no goal or side, so whether orange attacks −y cannot be told from it; runs were a few minutes, and the bot's log has only startup and exit lines, so it cannot show stalls. This does not bear on the os error 997.

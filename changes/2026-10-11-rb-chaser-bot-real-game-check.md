---
category: Changed
changelog: `rb_chaser_bot` ran on the real game for a short check (blue verified; orange direction, goals and long runs not).
---
## 2026-10-11 - rb_chaser_bot: first real-game check

- **Verified on the game:** Windows 11, RLBot v5, Freeplay map. The bot printed the right car and team for blue (car 0, team 0) and orange (car 1, team 1), ran with no connection error, and exited cleanly at match end; the blue car was seen driving at the ball, boosting and chasing it.
- **Docs:** the crate README notes that RLBot v5 joins `root_dir` onto the manifest's folder, so it must stay relative.
- **Known limitations:** the orange car's attack direction was not watched; the map has no goals, so chasing after a goal is untested; runs were a few minutes, and the bot's log has only startup and exit lines, so it cannot show stalls. This does not bear on the os error 997.

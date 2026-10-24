# Raw clips

Whole, untrimmed recordings from the owner's BakkesMod capture sessions
(2026-09-06 to 09-10), imported with the excerpts in the parent directory
from `baileyrd/rusty_bullet`'s `claude/rocket-league-server-clone-u74q45`
branch (head `bcf8f69`), byte for byte. No test reads them. They are here
so a later investigation (input recovery, the wall curve, dodges, goal
entries) can run `rb-verify --self <clip>` or a per-tick trace against a
full human recording without a new session. Over a whole clip the port
drifts far from the recording (chaos compounds over tens of seconds); the
one-step error (`rb-verify --self-onestep`) is the useful number there.

| Clip | Frames | What it holds |
|---|---|---|
| `groundjumpthrottle03.jsonl` | 3020 | throttle drive, tapped and held jumps, a landing, a drive up the back wall beside the goal, a brake and a handbrake turn |
| `walldrive04.jsonl` | 4626 | steered driving, a boost run at the cap into the `+X` wall curve, a reverse descent, a second wall climb, a wall jump and a backflip (recorded with no `jump` press) |
| `curverun05.jsonl` | 5677 | a lap riding the side-wall fillets and all four corner arches with boost and handbrake taps |
| `onewheellanding06.jsonl` | 5350 | many short hops with air roll, diagonal dodges, one-wheel landings |
| `hittickjump01.jsonl` | 5038 | boost, jump at the cap, an airborne hit, a second press with an all-zero stick; analog axes zero throughout |
| `hittickjump01_v2.jsonl` | 10114 | the third session's re-recording of the drill with the rebuilt plugin: three wheels-down hit-tick jumps, the cleanest cut as `hit-tick-jump` |
| `hittickjump01b.jsonl` | 3131 | boost, jump at the cap, the airborne hit `airborne-hit` is cut from, the ball's goal entry; analog axes zero throughout |
| `wall_curve02.jsonl` | 6825 | five wall-climb events (full-boost and throttle-only, both side walls); the first is `wall-climb-crest` |
| `goal_shot03.jsonl` | 9356 | a ground shot into the `+Y` net (`goal-shot-net-entry`) and a slow rolling shot the length of the field into the `-Y` goal |
| `clean_dodge04.jsonl` | 3925 | nine ground jump-dodges on open floor, including a pure-left and a mirrored pure-right one |

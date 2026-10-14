# Live test of `rb_rl_bridge` against the real game, 2026-10-12

What `rb_rl_bridge` did against Rocket League (Epic, Windows) around demolitions, a map reload, goals
and a pause. Times are the machine's local clock. Tested at `fc4bdceb1` (PR #631, now on `main`).
These are the author's reported observations from that machine and those runs, not independently
reproduced results. Private recordings were unavailable for review; where a cause is a guess, it says so.

## Summary

- A demolition keeps its actor: the game reports the car as Demolished (`PlayerPhysicsChange`
  state 5) for 3.00 s, then the same actor is back. 18 episodes in one run, no destroy and no
  respawn of a player actor. Between bots only; a demolition of the human's car was not observed.
- The bridge logged `BroadcastHeld` and `BroadcastResumed` only around map loads, never around a
  demolition, a goal, a kickoff or a pause.
- What lifts a hold after a map load is not established (see [Held and Resumed](#held-and-resumed)).
- The census found no tag outside the known unions. This run also saw tags the Freeplay slice never
  had: 6, 13, 14, 15, 16, 17 from the game and `SpawnBot`/`PlayerInput` towards it.
- The usual "start the game, read its log, close it, relaunch" recipe got stuck at the splash twice,
  and RLBot's kill-before-login order fixed it. That fits a single-use login code; the code's
  single use itself was not tested (see [Launching](#launching)).

## Launching

On Epic, `rb_rl_bridge --launch` cannot work: it has no login arguments, so the game never connects.
The recipe in `rb_rl_game_bridge/README.md` (start through Epic, read the log, close the game, relaunch
with `-rlbot ...`) got stuck twice at "click to start": the click made its sound but the splash never
advanced. RLBot's `Epic.Windows.cs` suggested a different launch sequence. RLBot starts the game through
`com.epicgames.launcher://apps/...?action=launch&silent=true`, and **kills it the moment the new
`Init: Command line:` line appears**, before the game logs in, then relaunches with that line's
arguments. In the stuck runs the first game had reached the menu and logged in (`Eos login result
'EOS_Success'` in `Launch.log`), and the relaunch's log showed no sign-in at all. The likely reason
is that the code was already spent. With RLBot's order the game reached the menu and a match in every
run where it launched at all.

Facts that cost time:

- `Launch.log` is under `OneDrive\Documents\My Games\...` on this machine, not plain `Documents`.
- In RLBot mode the menu shows "You are not connected to Epic Online Services. Would you like to
  reconnect?" Answering No keeps the run offline.
- Later, launches began to fail: `RocketLeague.exe` ran 7 to 14 seconds and exited, leaving an empty
  `Launch.log`, no crash dump and no Windows event, even for a by-hand launch of the same URL. Two
  guesses were tried and neither is confirmed as the cause: polling `Launch.log` without delete-sharing
  while the game renames it (the poll now opens it the way RLBot's reader does and the next run
  worked, but a later one failed again), and OneDrive not running (starting it changed nothing). A
  stale Epic launcher session after many idle hours is the remaining guess; it was not tested.

## Menus and match setup

- **Freeplay has demolitions disabled.** An offline Exhibition match with Mutators, Demolish, "On
  Contact (FF)" does.
- The Exhibition **pause menu does not pause the match**: the clock kept running behind it and the
  game kept streaming. Pausing and resuming produced no hold.
- After a match ends (a goal in overtime), the game rolls straight into a new match on a new map and
  shows Choose Team again.
- No AI cars of the game's own appeared in the lineup in RLBot mode; only the bridge's bots (and the
  human) did.

## Demolitions

One automated run (two bots, `On Contact (FF)`), read with `rb_rl_demo_count`:

| Map | Demolitions | Player spawns | Player-actor destroys |
|---|---|---|---|
| 1 | 10 (5 per bot) | 2 | 2 (at the map change) |
| 2 | 8 (4 per bot) | 2 | 0 |

Every episode lasted 3.00 s and ended with the same actor reporting a non-demolished state. Both bots
were demolished at the same instant each time. The only player `ActorDestroyed` messages
came when the match was left. Across 3 maps in another run there were exactly 2 `PlayerSpawn` per
map. Actor ids repeat on every map (38 and 39), and a bot's `command_id` is 1, 2 on the first map and a
large number on later ones (for example 50509/50510, 15551/15552).

The bridge log has no `BroadcastHeld` or `BroadcastResumed` inside any demolition. The wall clock maps
to the game's clock closely: first `PlayerSpawn` at game time 64.7 s was Create Match plus about 3 s.

**Not observed:** a demolition of the human's car, or of a car by the human. That is still open.

## Held and Resumed

| Run | Held | What happened before | Resumed | What happened before |
|---|---|---|---|---|
| manual | a pair at the Freeplay-to-Exhibition map change | not timed (the log had no timestamps yet) | the pair's resume | not timed |
| manual | 18:43:21 | not known; the match was probably ending (inferred from the later runs) | none logged before the game closed | |
| second | 20:04:04.584 | Leave Match confirmed 20:04:03.617 | 20:05:09.526 | Create Match 20:04:25.337, Spectate clicked 20:05:08.498 |
| second | 20:14:08.351 | overtime goal ended the match; new map, Choose Team | 20:14:36.375 | Spectate clicked 20:14:35.387 |
| scripted | 20:55:55.864 | Leave Match confirmed 20:55:55.060 | 20:56:09.716 | Create Match 20:56:06.425; Spectate clicked later, 20:56:24.180 |

Where it was timed, the hold followed leaving a match by about a second, and the roll-over's by a
moment (the exact gap was not measured). What ends it is not settled. In the second run
it came about a second after the human chose a team, and after 44 s on Choose Team. In the scripted run
it came 3.3 s after Create Match, before any team choice. So choosing a team is not required; whatever
else readies the new match (cars, a phase change) is the likelier trigger. An unattended run that
never chooses a team was not tried to the end (the planned `-ThroughMatchEnd -NoClickAfterRollover`
run could not start, see above).

`MatchStarted` races the Choose Team screen. After Create Match it appeared about 3 s later in one run
and never in another, where the game sat on Choose Team with 0 cars and a single `Paused` state change
although the bridge had sent two `SpawnBot` commands. The scripts spectate either way and then insist
on `MatchStarted`.

## Match phases seen

`Inactive`, `Paused` (before each Countdown, and at each map load), `Countdown`, `Kickoff`, `Active`,
`GoalScored`, `Replay`, `Countdown`, `Kickoff`, `Active`, ..., `Ended`, `Inactive`, `Paused`, and on
into the next match. Nothing unexpected in the goal, replay and kickoff cycle.

## Census

The author reported 117,869 inbound **framed bundles** over 3 maps, 0 unreadable.
The inbound tag counts below sum to 224,376 **event wrappers** inside those bundles.
These are different units: multiple wrappers can occur in one bundle, so the different totals
do not demonstrate a counting bug.

| Tag | Count | Tag | Count |
|---|---|---|---|
| 1 GameSpawn | 3 | 12 BallHit | 100 |
| 2 GameStateChange | 43 | 13 TeamScoreChange | 5 |
| 3 GameTimeChange | 470 | 14 ActorDestroyed | 83 |
| 4 GoalSpawn | 6 | 15 PlayerStatChange | 186 |
| 5 PickupSpawn | 102 | 16 PlayerStatEvent | 303 |
| 6 PickupStateChange | 261 | 17 PlayerSpectate | 39 |
| 7 PlayerSpawn | 6 | 18 PlayerInputChange | 23,261 |
| 8 PlayerPhysicsChange | 130,750 | 21 BallExtraDataChange | 8 |
| 9 PlayerBoostChange | 5,350 | RLBot to game: 1 SpawnBot | 6 |
| 10 BallSpawn | 8 | RLBot to game: 3 PlayerInput | 151,210 |
| 11 BallPhysicsChange | 63,392 | | |

Tags outside the known unions: none. The committed Freeplay slice
(`tests/fixtures/live_freeplay_slice.bin`) holds only tags 1 to 5, 7 to 12, 18 and 21, so 6, 13, 14,
15, 16 and 17 (and the commands in the other direction) are new to the evidence. `PlayerStatEvent`
(303 event wrappers) is not decoded; whether any of them mark demolitions was not checked.

## Open

- A demolition involving the human's car.
- What lifts the hold at a roll-over with no input at all.
- Why launches began to fail late in the day (see Launching).
- Decoding `PlayerStatEvent`.

Tooling for repeating this: [`tools/rl_live_test`](../../tools/rl_live_test/README.md) and
`rb_rl_demo_count` in `rb_rl_game_bridge`. The recordings contain the human player's display name
and are not committed.

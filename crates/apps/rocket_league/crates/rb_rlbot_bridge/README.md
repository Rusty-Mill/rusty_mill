# rb_rlbot_bridge

`rb_env` as an RLBot bot (stage 5 of the native RLBot port, `../../rlbot/PLAN.md`).

`rb_env`, the chaser and the verifiers all speak `PhysicsFrame` in and `ControllerInput` out. This
crate puts the real game on the same footing, so a policy written against a simulation plays in
the game unchanged:

| | |
|---|---|
| `observation(&GamePacket) -> Option<PhysicsFrame>` | what `Env::reset` takes; `None` when the packet has no ball |
| `controller_state(&ControllerInput) -> ControllerState` | unset stick axes centred |
| `Policy` / `PolicyBot` | `act(&PhysicsFrame, car) -> ControllerInput`; any `FnMut` is one; runs once per physics frame, a repeated packet gets the previous input |
| `run_policy(connection, settings, make)` | connects, handshakes and plays `make(init)` for every car core gives us |

```rust
let mut connection = Connection::connect(&env.server_addr)?;
run_policy(&mut connection, settings, |init| {
    let team = init.team as usize;
    move |frame: &PhysicsFrame, car: usize| chase(&frame.cars[car], &frame.ball, team)
})?;
```

- **Conventions.** Cars keep the packet's order (a car's index is its `player_index`), units are
  the game's (uu, rad/s, boost 0 to 100), and rotations go through `rb_scenario::rotator_to_quat`,
  which was checked against a recorded quaternion. The timestamp is the match clock.
- **What a policy cannot see.** Only what a `PhysicsFrame` holds: no boost pad timers, score or
  match clock beyond the timestamp, and no previous inputs (`input` is `None`).
- **Not covered.** Nothing here has been run against the real game. The tests check the
  conversions (a quarter turn of yaw points the nose along +y; the chaser drives straight at a
  ball it faces) and the per-packet decision; the client's own tests cover the packet loop.

## Chaser bot

`chaser(team)` is `rb_env`'s scripted chaser as a `Policy`; the `rb_chaser_bot` binary plays it.
`chaser.bot.toml` is the RLBot manifest (its `root_dir` is the repo root):

```
cargo build --release -p rb_rlbot_bridge --bin rb_chaser_bot
```

Then add `chaser.bot.toml` in the RLBot GUI and start a match. RLBot launches the binary with
`RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID` set (the port defaults to 23234 when unset); `RLBOT_SERVER_ADDR` overrides the full `host:port`.

RLBot v5 joins `root_dir` onto the manifest's own folder, so keep it relative (an absolute path
there fails to launch the bot).

**Real-game check (Windows 11, RLBot v5, short sessions).** On Freeplay the bot printed `car 0 on team 0`
as blue and `car 1 on team 1` as orange, ran with no connection error, and exited cleanly; both
cars were seen driving at the ball, boosting and chasing it. In a soccer match with kickoff and
assigned goals (orange bot vs a Psyonix bot, about 3 minutes) orange attacked the blue goal, none
went into its own, the right team's score rose, and it kept chasing after the goal and reset.
Not established: blue's direction in a soccer match, anything over a long run, and stalls (the bot
logs only its startup and exit lines, and no goal count was taken).

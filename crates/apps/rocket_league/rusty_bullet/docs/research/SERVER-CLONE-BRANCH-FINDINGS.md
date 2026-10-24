# Findings from the server-clone branch

`baileyrd/rusty_bullet` (archived) has one branch ADR-0008 left unimported:
`claude/rocket-league-server-clone-u74q45`, 49 commits from 2026-09-04 to 09-10, head
`bcf8f69`. Despite the name it holds no server or netcode work. It is a physics line that
forked from `main` before the wheel-ray work and reused the numbers `RB-PHYSICS-001-FR-079`
to `FR-094` for different findings than `main` gave them.

The owner chose (board card w3, 2026-10-24) to import its captures and record its
findings, not its code or history. The captures are in
`crates/rb_capture_ingest/fixtures/human/`, gated by `rb_verify_cli/tests/human_captures.rs`.
This note maps each branch finding to where `main` now stands.

**How to read it.** "Branch FR" is the branch's own number. Write it as `branch FR-0NN` if
you cite it, because `main`'s `FR-0NN` is a different requirement. "On main" names the
`RB-PHYSICS-001` requirements that cover the same mechanism, matched by topic. "Evidence"
is main's error on 2026-10-24 on the branch's own capture of that mechanism (car mean / max
position, uu, from `human_captures.rs`). For comparison, the branch's own code measured
364 uu mean on `clean-dodge` and 169 uu on `wall-climb-crest`.

## Physics findings

| Branch FR | What the branch found | On main | Evidence on main |
|---|---|---|---|
| 079 | A dodge derails a capture: air-control torque ignored inertia, and pitch/roll signs were flipped | FR-082, FR-084 | `dodge-derailment` 7.0 / 24.1 |
| 080 | The real dodge: `FLIP_INITIAL_VEL_SCALE`, continuous flip torque, pitch-held flip cancel | FR-069, FR-070, FR-083, FR-085 | `clean-dodge` 1.8 / 4.8 |
| 071 | Real air-control damping replaces a landing assist | FR-084, FR-097 | `airborne-hit` 1.5 / 8.6 |
| 081 | Hitbox at the Octane offset; dodge impulse along the car's horizontal axes | FR-078, FR-082, FR-090 | `dodge-derailment` as above |
| 082 | Wheels: raycast suspension, tire impulses, tire curves, analog handbrake, auto-roll, wall-jump normal from the wheels | FR-088, FR-090, FR-099, FR-101, FR-131, FR-132, FR-134 | `throttle-jump` 1.9 / 5.6; `dodge-free-landing` 1.1 / 3.1 |
| 083 | After a hit: car-ball pair material and RocketSim's extra hit impulse | FR-107, FR-125, FR-137 | `airborne-hit` ball 1.5 / 8.5 |
| 084, 087 | Landing and jump exit: wheel ray reach, the stick gate, wheels-down hit-tick jumps | FR-092, FR-129, FR-138, FR-150 | `hit-tick-jump` car 1.6 / 7.5 (ball 189 / 1024 after the hit: open on main too) |
| 085 | Second capture session: the speed cap, goal fillets, corner arches, the press tick | FR-102, FR-106, FR-112, FR-113, FR-117 | `boost-wall-entry` 0.9 / 9.5 |
| 086 | The wall curve's fillet radius (270 uu) and a pushback without its positional term | FR-102 (real transition radii, from RocketSim's meshes), FR-122 | `boost-wall-entry` as above |
| 088 | The wall-climb speed gap: the sessions were recorded with **unlimited boost** in freeplay, so the recorded car never drained | FR-111 (unlimited boost as a drain rate); `rb_verify_cli::boost_is_unlimited` | `wall-climb-crest` 17.0 / 91.0 |
| 089 | A ball through the goal mouth hit a phantom back-wall fillet | FR-112 (the goal mouth is open), FR-113 | `goal-shot-net-entry` ball 6.0 / 20.9 |
| 090 | A pure side dodge splits its flip torque between two body axes (`FLIP_TORQUE_Y`) | FR-083, FR-085 | `clean-dodge` as above |
| 093 | The wheels act one or two ticks past the jump | FR-092, FR-095 | closed on the branch too |
| 094 | A 38° dodge-direction miss after held air control (four dodges) | FR-072 to FR-074, FR-103, FR-147 | `clean-dodge`, `dodge-derailment` as above |
| 085 J | After `onewheellanding06`'s first dodge hop the port stayed upside down | closed | reseeded at 6.2 s (`rb-verify --scenario-from`), main lands upright and stays within 34 uu of the recording for 2.5 s |

Nothing in this table needs new code on main. If a later capture shows one of these
mechanisms wrong again, start from the branch commit named in its row of the branch's
`RB-PHYSICS-001` change history, not from its code: main's port has diverged too far for a
patch to apply.

## Capture findings

These are about the recordings, not the physics, and are new to main. They also apply to
any later BakkesMod capture made the same way. `RB-VERIFY-002` points here.

- **Unlimited boost.** Every capture in the September freeplay sessions was recorded with
  unlimited boost on (owner's confirmation, branch FR-088). `boost_amount` does not drain.
  `rb-verify --self` already turns the drain off when `rb_verify_cli::boost_is_unlimited`
  detects this.
- **Single-tick spikes.** A recorded `angular_velocity` can jump 3 to 8 rad/s for one tick
  while the car is airborne and its quaternion moves smoothly (branch FR-091). A recorded
  input axis can drop to zero for one tick next to an event: `dodge-derailment`'s second
  dodge records `pitch = 0` for a velocity change that is plainly diagonal (branch FR-092,
  first session, before the plugin was versioned). Check a surprising one-tick value
  against its neighbours before treating it as physics.
- **Dead axes and a missing press.** `hittickjump01` and `hittickjump01b` record every
  analog axis as zero; `walldrive04` has a dodge at 30.175 s with no `jump` press recorded.
- **Seeding inside a held jump.** A world seeded while jump is held reads the held button as
  a new press and fires a second jump (branch FR-094). Seed before the press. The default
  seed (`SeedFrame::FirstGroundedNeutral`) already requires jump released; `--seed-first-frame`
  does not.

## Not imported

The branch's code (`rb_physics_bullet`, `rb_verify_cli`, capture plugin 1.1) and its
history. Main's capture plugin is at 1.5 and records the same per-car `ControllerInput`.
The branch stays readable in the archived repository; clone it anonymously to read a
commit.

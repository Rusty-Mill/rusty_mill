# Rocket League mechanics catalogue

Working list for `RB-RESEARCH-O006`: every named mechanic found so far,
what it stresses in the physics model, and how it could be detected in a
replay or capture. Names and one-line descriptions come from community
sources (Liquipedia glossary, Dignitas, GameRant, EarlyGame, rocketprices,
Harmonicode: see links below) and the owner's own play; the "stresses" and
"detect" columns are this project's reading and are unverified until a
capture exists. Status: `Captured` (a recording in the owner's set shows
it), `Modelled` (the port has the mechanic), `Needs capture`.

## Ground movement

| Mechanic | What it is | Stresses | Detect | Status |
|---|---|---|---|---|
| Drive / powerslide | Throttle, steering, handbrake drift | tire grip, handbrake curves | steering and handbrake inputs, lateral slip | Captured, Modelled |
| Boost | Forward thrust, minimum 0.1 s burn | boost timer, air vs ground accel | boost input vs velocity gain | Captured, Modelled |
| Coasting / braking | Release throttle, brake | non-sticky friction | throttle 0 | Modelled |
| Wall ride / driving on walls and ceiling | Stick to surfaces | sticky force, wheel contact | 1-4 wheel contacts on non-floor normals | Captured (walls), Modelled |
| Powershot / wall drive | Hit ball while on wall | car-ball contact on slopes | ball touch with car z-up not +z | Needs capture |

## Jumps and flips

| Mechanic | What it is | Stresses | Detect | Status |
|---|---|---|---|---|
| Jump (held for height) | 0.2 s hold accel | jump hold, impulse | jump input timing | Captured, Modelled |
| Double jump | Second jump, no direction | double-jump impulse | two jump presses, no flip torque | Modelled |
| Flip / dodge (front, back, side, diagonal) | Second jump with direction | flip impulse, flip torque, damping | pitch/yaw/roll inputs at second jump | Captured, Modelled |
| Flip cancel | Cancel the flip rotation | flip torque timing | opposite pitch input after flip | Needs capture |
| Stall | Tilt car to cancel a flip mid-air | flip torque, air control | roll plus pitch at second jump | Needs capture |
| Wall jump / wall dodge | Jump off a wall, flip off it | wall contact refills double jump | jump with wall contact | Modelled |
| Auto-flip (roof recovery) | Jump while on roof flips car over | auto-flip | roof contact plus jump | Captured, Modelled |
| Speed flip | Diagonal flip cancelled with air roll and boost, fastest kickoff | flip torque, air control, boost | kickoff diagonal flip, roll, boost | Needs capture |
| Wave dash | Flip on landing to keep speed | landing with flip, tire grip, pushback | flip within a few ticks of ground contact | Needs capture |
| Half flip | Backflip then cancel and air roll to turn | flip, air control | back flip then reverse pitch | Needs capture |
| Fast aerial | Jump, boost, second jump for height quickly | jump hold, boost | jump, boost, jump in sequence | Needs capture |
| Fast kickoff | Boost plus flip off the line | flip, boost, ball contact | first seconds of kickoff | Captured (kickoff) |

## Aerial control

| Mechanic | What it is | Stresses | Detect | Status |
|---|---|---|---|---|
| Air roll | Roll about forward axis | air torque | roll input | Modelled |
| Air control | Pitch/yaw/roll while airborne | air torque, damping | inputs while no wheel contact | Captured, Modelled |
| Recovery / landing on wheels | Orient before landing | auto-roll | touching ground in air | Captured, Modelled |
| Boost aerials | Boost along car forward | air boost | boost while airborne | Captured, Modelled |

## Ball contact

| Mechanic | What it is | Stresses | Detect | Status |
|---|---|---|---|---|
| Dribble | Carry the ball on the roof | car-ball friction 2.0, contact persistence | long ball contact, ball z above car | Needs capture |
| Flick (incl. musty flick) | Flip through the ball from a carry | car-ball hit, extra hit impulse | flip while ball contact | Needs capture |
| Air dribble | Repeated air touches | many small car-ball hits | consecutive touches in air | Needs capture |
| Double tap | Hit ball off the wall, then again | wall bounce, car-ball | two touches around a wall bounce | Needs capture |
| Ceiling shot | Drop from ceiling holding the dodge | ceiling contact, flip availability | ceiling plus later flip touch | Needs capture |
| Flip reset | All four wheels touch the ball in the air, refunding the flip | wheel-ball contact, jump re-arm | four wheel contacts on ball | Needs capture; wheel-ball Modelled (rays), reset not |
| Pogo | Bounce off the ground with a corner of the car, springing back up | hard landings, pushback, jump on contact | repeated ground contact plus jump within a few ticks | Needs capture (see residuals in PROJECT-STATUS) |
| Redirect, bump, demo, 50/50 | Contact play | car-ball, car-car, demolition | contacts, demos | Bumps and demos not modelled |
| Bicycle hit, juggle, clear, center, epic save | Statistic-level labels | various | ballchasing detail stats | n/a |

## Sources for the names

[Liquipedia glossary](https://liquipedia.net/rocketleague/Liquipedia:Glossary),
[Dignitas: wave dash and half flip](http://dignitas.gg/articles/blogs/rocket-league/12676/wave-dash-half-flip-and-wall-positioning-useful-mechanics-for-recovering-quickly-),
[Dignitas: five new mechanics](https://dignitas.gg/articles/five-new-mechanics-to-spice-up-your-game),
[GameRant maneuvers](https://gamerant.com/rocket-league-best-maneuvers-guide/),
[EarlyGame](https://earlygame.com/rocket-league/most-important-rocket-league-mechanics),
[rocketprices freestyle guide](https://www.rocketprices.com/news/983--rocket-league-freestyle-shot-guide--how-to-ceiling-shot-double-tap-air-dribble-flip-reset),
[10 flip resets](https://gamersrdy.com/blog/2021/02/03/10-flip-resets-you-need-to-learn-in-rocket-league/),
[Harmonicode dictionary](https://harmonicode.com/2026/03/25/rocket-league-terms-the-complete-dictionary-every-player-must-know-in-2026/),
[ballchasing API](https://ballchasing.com/doc/api).

## Gaps to fill next

- Fetch the Liquipedia glossary in full and diff it against this list.
- Add variants (e.g. the ten flip-reset kinds, flick variants, kickoff
  types) as sub-rows once each has a detector.
- For each row marked `Needs capture`, define the minimal capture (what
  to do, how long) so a bot or the owner can record it.

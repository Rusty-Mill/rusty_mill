# Rocket League physics: Rocket Science + Psyonix GDC 2018

Sources
- **RS-KB** = Rocket Science (HalfwayDead) written transcripts on rocketscience.fyi/know: #1 boost & jump (2016), #12 bounces (2018), #14 dodges (2018), #20 shot power (2019), plus the "(Un)known issues" page. Highest confidence (text, with the author's later corrections).
- **RS-CC** = Rocket Science episodes pulled as YouTube auto-captions (21 more episodes, section 5). Auto-captions can garble digits; every number there is flagged where I think it is wrong or unverified.
- **GDC** = Jared Cone, "It IS Rocket Science!" (GDC 2018). Slide deck text (media.gdcvault.com) + Glasp summary of the talk video (ueEmiDM94IE).
- **3P** = third party, not requested, used only to fill a gap (smish.dev ball_simulation_3, with RLBot/Nevercast/tarehart). Marked where used.

Units: uu = Unreal units. Ticks are physics ticks at 120 Hz (8.33 ms).

## 1. Parity-table candidates (one row per constant)

| Parameter | Rocket Science | GDC |
|---|---|---|
| Physics tick rate | 120 Hz (#12) | 120 Hz, 8.33 ms, fixed (slides) |
| Engine | - | Bullet, open source, modified, single-threaded, ~1 week integration |
| Collision detection | Discrete; suggests CCD as a fix (#12) | Discrete, chosen for performance |
| Penetration handling | Tick overlap uses wrong normal on curved meshes (#12) | Larger step = larger penetration = inconsistent hits; 120 Hz chosen to reduce; Cone wishes for a different fix |
| Max car speed | 2300 uu/s (#14) | - |
| Jump hold window | 200 ms of extra upward force along car roof; 2nd jump has no hold force (#1) | - |
| Dodge timer | 1.25 s, starts after the jump hold ends; full hold gives 1.45 s total (#14) | - |
| On-ground test | >=3 wheels touching = grounded; flip available with 0-2 wheels (#14) | - |
| Dodge duration | 0.65 s (#14) | - |
| Dodge vertical cancel | 18 ticks (0.15 s) untouched, then vertical speed -35%/tick; upward case only 8 ticks (to ~3%); downward settles near 15 uu/s (#14) | - |
| Dodge impulse, forward | +500 uu/s, horizontal only (#14) | - |
| Dodge impulse, side | 500 uu/s base, scales linearly with forward speed up to x1.9 at 2300 uu/s (#14) | - |
| Dodge impulse, backward | 533 uu/s base, x2.5 scale; up to 1333 uu/s of braking; direction defined by current travel direction (#14) | - |
| Dodge torque | Applied every tick for 0.65 s; reaches max spin in 3 ticks (25 ms) (#14) | - |
| Flip cancel | Only pitch-roll part cancelable; proportional to stick; ignored in first 5 ticks (40 ms window to move stick) (#14) | - |
| Boost (small pad) | Exactly 12% (old 30/255 = 11.76%) (#1) | - |
| Air throttle | Forward and backward force exist in air; weaker than boost; backward weaker than forward; cannot cancel gravity (#1) | - |
| Ball-car hit | Real engine impulse (perfectly inelastic + friction) plus Psyonix extra force along car-center-to-ball direction, scaled by relative speed (#20) | Not covered; Corey Davis GDC 2016 is the source |
| Vehicle friction | - | Lateral only, no longitudinal; see section 3 |
| Mass | - | Constant, ignored when applying forces |
| Transmission | - | None; force/accel curve instead |
| Determinism | Cross-CPU physics fixed in patch 2.53 (issues page) | Fixed tick for determinism |

## 2. Rocket Science detail

### #1 Boost & jump (2016; parts marked outdated by the author)
- Hold-jump force pulls toward the car roof, active only the first 200 ms (corrected from 250 ms). The second jump has none.
- Jump impulse acts along the roof direction, so pitching back 90 deg before the second jump wastes it.
- With 0 boost: hold ~0.25 s, pitch ~10 deg back, second jump, then pitch fully back and keep air-throttling. With any boost: pitch ~30 deg back, boost, jump immediately so the second jump's momentum carries over.
- Reference heights (stated as not an upper bound): crossbar with 5 boost, above the goal with 12 boost, standard-map ceiling with 40 boost. Full boost reaches the ceiling in a little over 2 s.

### #12 Bounces (2018)
- True ball state is computed at 120 Hz. Each tick: contact test, then bounce from linear and angular velocity and incoming angle vs surface normal.
- Flat walls are fine unless the ball would cross the wall within one tick.
- Curved or multiple adjacent meshes: the ball is already inside the wall when the bounce is solved, so the normal is wrong. Posts are the worst case; crossbar has the same bug.
- Author raised tickrate 120 to 500 as a test and the bad post bounces resolved.
- Proposed fixes: more ticks (costly; most games use 60 Hz) or CCD, solving for the first-contact time inside a tick (may conflict with a same-tick car hit).
- Wasteland posts are 90 deg corners and behave best; author suggests uniform corner-style posts on all maps.
- Patch 1.41 made no physics changes. Psyonix improved inward-curve bounces about a year earlier by unknown means.

### #14 Dodges (2018, revised 2026)
- Preconditions: stick past deadzone and dodge deadzone (author's tool: dz.rocketscience.fyi). In-game air sensitivity does not affect dodges; external tools can.
- Three components: impulse, torque, vertical momentum cancel.
- Impulse direction is the top-down projection of the car's forward axis (so left/right swap when the car is tilted back far enough). Never vertical.
- Diagonal dodge: split into forward and side components; forward fixed, side scaled by forward speed, so the true impulse angle differs from the stick angle. Optimal example at 1700 uu/s: car turned 60 deg, stick angle 52 deg, giving 600+ uu/s gain (reaches max speed). At ~100 uu/s: 55 deg and 55 deg. Calculation ignores boosting during the dodge.
- Flip cancel details and half-flip: cancelling a straight flip fully can take almost 1 s and rotates ~100 deg more, so diagonal half-flips cancel faster.
- Perfect no-flip needs an opposing torque (for example a ball touch at the instant of the dodge), backward angular momentum before the dodge, or air roll to rotate the axis (the speed flip).
- Stall: dodge with zero direction cancels vertical momentum only (needs an air roll binding opposed by steering).

### #20 Shot power (2019)
- Model: normal engine collision (perfectly inelastic, friction, equal contact-point velocity) plus Psyonix's extra force along the car-center-to-ball direction, growing with relative speed.
- Stationary ball example: only ~40% of the force came from the Psyonix part.
- Redirects and lateral passes can exceed 140 kph; regular power shots top out near 120 kph.
- Moment of inertia explains the corner-hit advantage: the hit spins the car, so the ball needs less speed to match contact-point velocity.
- Dodge rotation adds at most ~1% over a perfectly aligned no-dodge shot (tested on the longest car). Diagonal dodge lever adds ~1%. Diagonal gains slightly more speed than straight only below 78% of max speed.
- Dodging mainly helps through the speed gain. Flip cancel generally gives slightly less power; no effect on the shot if contact is within 50 ms of the dodge.
- Flicks: each extra touch adds speed; one-touch flicks are never powerful. Octane vs Breakout: ~5% difference on two-touch flicks. Centers of mass sit toward the rear on all cars, so backflip flicks give more roof length.

### Issues page (physics and netcode)
- Double touch on one shot: 56 to 94 to 105 kph frame by frame; each touch adds energy, so energy is not conserved.
- Demo rules avoid surface-angle math because of discretization error; a fix likely needs CCD.
- Cars of the same preset spawn at slightly different positions (mesh spawns first, hitbox attached after).
- Server does not send flip-reset availability, causing failed-reset warping. Kickoff countdown ignores ping.

## 3. GDC detail

### Physics engine
- Goals: fast responsive vehicles, consistent controllable physics, competitive over the internet.
- Bullet added alongside UE3's physics for gameplay physics only. Boxes moving in about two days, vehicle prototype about a week later.
- Fixed 120 Hz tick for determinism; slower rate = larger penetration = inconsistent impact normals and ball trajectories. Higher rate costs more, especially for network corrections.

### Vehicle tuning
- Goals: fast acceleration and braking, sharp steering, stable driving, fast recovery.
- Coupled variables: torque, tire/spin friction, wheel radius, suspension, gravity, mass, gears. Simplifications used:
  - No transmission, force/accel curve.
  - Constant mass, ignored when applying forces.
  - No longitudinal friction; lateral friction only:
    - Ratio = SideSpeed / (SideSpeed + ForwardSpeed)
    - SlideFriction = Curve(Ratio)
    - GroundFriction = Curve(GroundNormal.Z)
    - Friction = SlideFriction * GroundFriction; Impulse = Constraint * Friction
- Friction application point causes tipping. Workarounds on the slides: lower friction, limit steer angle, "stay upright" constraint, apply forces at CM height. Separate "stability forces" slides are diagram-only.
- Wheel positions (axle width, axle separation) strongly affect handling; a 2 cm change is shown. Moved to a preset system: collision box size and offset, wheel positions, radii; many vehicles share few presets. Physics and visuals therefore do not match exactly.

### Networking
- Rejected: input delay; server waiting for client input (jitter, de-sync on moving-object hits); server-side lag compensation.
- Server input buffer: client sends input every physics frame; buffer avoids pauses and blocks speed/jitter cheats but adds latency.
- Buffer control: upstream throttle (server tells client to run extra or fewer frames); downstream throttle (server consumes 0, 1 or 2 inputs per frame; minor desyncs).
- Prediction and correction: client records input and history by frame number; server returns frame number and physics state; client compares, and on a large difference reverts all physics actors to that frame and re-simulates forward to catch up.
- Client predicts everything including the ball (ball is predictable, cars less so). 200 ms ping at 120 Hz = 24 correction frames.
- Result: no input delay, high-ping clients do not ruin games, moving objects hit reliably, 100% server authoritative.

## 4. Where RS and GDC overlap or disagree
- Both: 120 Hz, discrete collision. RS proposes CCD; Cone treats discrete as a deliberate cost trade and regrets not finding another penetration fix.
- GDC covers vehicle model and netcode; it has no dodge, jump, boost or hit constants. RS has no suspension or friction constants.
- Both name the same root cause for inconsistency: penetration depth varying with tick size.

## 5. Additional Rocket Science episodes (auto-captions, RS-CC)

21 episodes pulled with yt-dlp captions (video IDs in section 8). Not covered (blocked or visual): Hitbox Visualizations (99j1mTN1_Vs), Curvedashing (PuB6yLVSs5s), input-lag/controller/BakkesMod/esports videos (not physics).

### 5.1 More parity-table rows (extends section 1)

| Parameter | Value (RS-CC unless noted) | Episode | Check |
|---|---|---|---|
| Unit | 1 uu = 1 cm | #4 | - |
| Gravity | 650 uu/s^2 (about 2/3 of Earth) | #4, #11 | consistent in two videos |
| Car max speed | 2300 uu/s, hard cap: speed vector is renormalised, never exceeded by any source (bumps, dodges, wavedash) | #11 | matches RS-KB #14 |
| Supersonic threshold | 2200 uu/s (trails, FOV +5 deg, demo threshold) | #11 | - |
| Boost tap | min 13 physics frames per activation; a 0.1 s extra tap after supersonic reaches 2300 | #11 | 13 frames unverified (could be 12) |
| Speed-keep while turning | tap boost every 24 ticks (Breakout), 27 (Dominus), 26 (all others) for ~2270 avg | #11 | - |
| Ball max speed | 6000 uu/s | #4 | - |
| Ball max spin | "60 RPM, one turn/s" (about 6 rad/s) | #4 | caption rounds; verify rad/s |
| Ball air drag | loses 3%/s of speed (halves in ~22 s); spin has no effect in air | #4 | 0.97^22 = 0.51, checks out |
| Ball ground slide friction | 230 uu/s^2; rolls without sliding below ~565 uu/s (one ball circumference per s) | #4 | 565 vs 2*pi*radius (about 573 at r=91.25) |
| Ball stop rule | speed < 40 uu/s for 2.5 s and spin < 10 RPM: ball stops; spin above 10 RPM prevents stopping | #4 | - |
| Ball bounce restitution | 0.6 of the normal component; tangential loss depends on friction and spin (shallower angle = less friction loss) | #4 | - |
| Car-ball hit direction | from car root point to ball centre, adjusted "as if the car were shorter", with upward component dampened (supersedes #4's claim of root-to-impact-point) | #6 (Corey Davis) | formula numbers only from 3P |
| Wheel hits | wheel-ball contact does not obey Newton's 3rd law: ball is pushed, wheels exert no reaction | #10 | - |
| Suspension rest | resting wheel position ~2 uu lower under normal gravity; stretches to 12 uu below rest in air; first 12 uu of compression on landing give no force | #10 | - |
| Flip reset | all four wheels touching the ball at once | #10 | consistent with RS-KB #14 (3+ wheels = grounded) |
| Car angular velocity cap | 5.5 rad/s total | Applied #2 | - |
| Air angular acceleration | yaw 9.11, pitch 12.46, roll 38.34 rad/s^2 (roll has a damping term); same for every car | Applied #2 | defers to smish.dev for damping math |
| Jump impulse | 292 uu/s along surface normal; second jump 292 uu/s along car-up | Applied #2 | caption says "uu/s squared"; it is a velocity |
| Jump hold force | 1458 uu/s^2 along car-up for up to 0.2 s (adds up to ~292 uu/s); active at least 3 ticks even on instant release | Applied #2 | caption's "3,600 uu/s" for the minimum is garbled; 3 ticks at 1458 is about 36 uu/s |
| Sticky force | 325 uu/s^2 toward the ground while wheels touch; wheels stay down for the first 6 ticks of a jump | Applied #2 | - |
| Air throttle | +/-66 uu/s^2 along car-forward; ~6% of boost strength; lets a vertical car cancel ~10% of gravity; backward weaker | Applied #2 | - |
| Octane rest height | root joint z = 17.01 uu | Applied #2 | - |
| Fast aerial | hold jump 0.2 s, release exactly one tick (8 ms), second jump asap; boost after tilting back; tilt back asap; a 2.125 s ceiling run vs 2.28 s earlier macro | Applied #2 | - |
| Boost with little left | most efficient time ~0.52 s after takeoff | Applied #2 | - |
| Engine acceleration | throttle accel ~58 km/h/s (about 1611 uu/s^2) at standstill, decreasing linearly to 0 at ~50 km/h (about 1389 uu/s); wheelie (rear wheels only) loses no acceleration above that speed | wavedash video | derived from kph; kph rounding makes these approximate |
| Dodge deadzone | dodge needs \|x\|+\|y\| >= 0.5 at 0 deadzone (tilted square); axis-independent deadzone; ~11.5 deg of each direction snaps to straight | #7, #7.1 | - |
| Hitbox presets | Octane, Dominus, Plank, Breakout, Hybrid (all cars of a preset identical in physics); Batmobile separate; Plank folded into Batmobile preset in patch 1.56; Merc added as own preset later | #8, 1.56, Merc | - |
| Hitbox placement | box placed relative to the root joint (pivot/CoM) with x and z offsets; wheel and suspension data per preset | #8 | - |
| Hitbox tilt | every car's hitbox is tilted at rest; octane back 1.22 uu higher than front (-100 internal units = 0.55 deg); caption says max 2 deg forward and 8 deg backward in one place and "none more than 2 deg" in another | Mini #1 | the 8 deg figure is internally inconsistent |
| Octane hitbox length | 118.01 uu | 1.43/1.44 | - |
| Merc | hitbox top 61.11 uu on wheels (10% above Octane), 9% narrower, 2% longer, CoM slightly less rearward and lower, reach 1% less | Merc | - |
| Boost pad pickup | cylinder on root point; small pad radius 144 uu; big pad radius caption says "28" (almost certainly a garble); heights ~168 big / 165 small; second box 160 (big) / 96 (small) half-extent horizontally, ~130 vertical, any non-wheel part, "lock-on" persists while touching it | Boostpad Mini #2 | verify big-pad radius |
| Heatseeker | after each touch ball speed set by touch count (not hit quality); curve from a force toward the target net centre | Heatseeker | - |
| Patch 1.43/1.44 | server compresses state; location and linear velocity now rounded to 2 decimals on both sides; cars never reach exactly zero velocity (~0.07-0.65 uu/s residue); rest pitch band -96 to -125 internal units (octane); back wheels moved slightly forward (turning +0.5-1%) | 1.43/1.44 | - |
| Patch 1.45 | hitbox angle at rest stabilises again; boost input lag +1 tick (8 ms) fixed | 1.45 | - |
| Boost internals | float 0-1 in game, 8-bit in replays; since Dropshot patch replay values round down | #6 | - |

### 5.2 Per-episode notes

- **#4 Ball physics (supersession)**: units, caps, drag, friction, bounce above. Cars interact only through a box hitbox; wheels have a separate hitbox acting like shock absorbers. Impact "normal" is not surface-perpendicular, so the ball cannot rest perfectly still on a roof; model the car as a sphere ("pool"); car about 10x ball mass. The "root-to-impact-point" detail was corrected in #6.
- **#6 Mistakes and additions**: real camera extra distance at supersonic with 0 stiffness is 114.7 uu (not 100); FOV depends on aspect ratio (true horizontal FOV only at 16:9); crates CI wording corrected; the hit-direction correction above.
- **#8 Car hitboxes and turning**: root joint = pivot = hit-direction origin; hitbox offsets; wheels/suspension per preset; stats measured at rest to 0.01 uu; turning measured via angular velocity read from memory.
- **#10 Wheel hits**: octane example: car at 2000 uu/s is slowed to 1300 uu/s by suspension before the hitbox touches, so the hit is weak; at ~290 uu/s the suspension stops the car before the hitbox touches and the ball is unaffected (credited touch, no deflection); front-wheel air dribbles push you down. Dominus has the strongest wheel hit (axle spacing and small wheels), then Breakout; Batmobile/Plank weakest.
- **#11 Supersonic != max speed**: see table; u-turns are not fastest at max speed.
- **#16 Netcode and lag**: server physics is truth; client predicts itself and the ball (deterministic physics) so 340 ping plays like offline if stable; other players' cars predicted as "inputs unchanged"; goals, ball-hits-floor game end, kickoff timer and boost-pad pickup are not predicted; client sends inputs (not button states) and the last four physics ticks of input per packet; server sends 60 packets/s; client send rate 65-70/s (about one per frame); in-game client/server send rate and bandwidth options appear to have no effect; scoreboard ping only correct at 60 fps; both sides keep a jitter buffer; server health vs packet loss icons explained.
- **#18 Best turning cars**: presets within 10% on early ticks, 3% on final turning speed (25% in 1.7 era); octane tightest at start, worst at end; break-even angles vs octane: Dominus 25 deg, Breakout and Hybrid 78 deg, Batmobile 107 deg; optimum turning speed 850-900 uu/s; after releasing input the car keeps turning 7.7 deg (Octane), Breakout 22% more; with counter-steer all stop in 2.5-2.9 deg; ranking for short turns: Dominus, Hybrid, Octane, Breakout, Plank, Batmobile.
- **#19 Input consistency / heavy car bug**: physics deterministic for the same inputs in the same tick; inputs sampled per frame, so action durations are quantised to whole ticks; FPS multiple of 120 gives best consistency (60 FPS cannot express odd-tick durations); polling and human timing sigma about 15 ms; physics ticks are queued relative to visual frames (unevenly) so reading input per tick does not help; heavy-car bug judged placebo/visual with no physics change found.
- **Applied #1 Dribbling**: ball must be placed to the side of CoM to supply centripetal force; roof gives less than 40% of max side force; a tall hitbox (Octane about 25% higher than Breakout at front) gives more time on bounce dribbles; preset difference in tightest dribble turn only 1%.
- **Landing wavedash / sonic flip**: tilt forward, tap a short jump when the last wheel touches (rear suspension still extended); works with no dodge left; success well above 99% claimed; worth it only if dodge delay is short (0.3 s at zero speed and zero boost).
- **NoFlip/Airdash**: bind "dodge forward"; hold it only while pressing jump, release before to avoid a backflip, hold too long gives a quarter flip.
- **Heatseeker, 1.56 Plank change, 1.43/1.44, 1.45**: in table.

## 6. Cross-checks between sources

| Item | Result |
|---|---|
| Tick rate 120 Hz | GDC slides, RS-KB #12, RS-CC #11 and #19 all agree |
| Car max speed 2300 uu/s | RS-KB #14, RS-CC #4 and #11 agree |
| Small boost pad 12% | RS-KB #1 and RS-CC #6 agree (11.76% before the patch) |
| Jump hold force 0.2 s | RS-KB #1, RS-KB #14 and RS-CC Applied #2 agree |
| Dodge timer total | RS-KB #14: 1.25 s plus 0.2 s hold = 1.45 s; wavedash video quotes 1.45 s |
| Hit direction | RS-KB #20 and RS-CC #6 agree; RS-CC #4 is superseded by #6 |
| Corey Davis GDC 2016 vs RS | RS-KB #20 quotes Corey describing "vector from ball to car"; #6 shows the real code adds length and vertical adjustments |
| GDC 2018 vs RS-CC 1.43/1.44 | RS cites the GDC talk as the reason for the 2-decimal rounding (compression of server state) |
| Ball spin cap | only one source (RS-CC #4); verify as rad/s |
| Boost pad radii | only one source (RS-CC Mini #2); big-pad "28" is a garble |

## 7. Gaps

- Still not transcribed: Hitbox Visualizations (99j1mTN1_Vs, mostly visual), Curvedashing (PuB6yLVSs5s), the rest of the 68 videos (input lag, controller tests, BakkesMod, esports, TAS). Full list: /tmp/rocket-science-channel-videos.md.
- YouTube auto-captions: digits can be wrong; flagged above. RS-KB pages are more reliable where they exist; only 4 episodes have them.
- GDC slides are text only; diagram slides and talk audio are missing.
- 3P (smish.dev): car-ball hit formula J = m_ball*|dv|*s(|dv|)*n with n = ball.pos - car.pos; n.z *= 0.35; n = normalize(n - 0.35*dot(n,car.forward)*car.forward), ball only; this is consistent with RS-CC #6's description (shortening adjustment plus vertical damping) but the 0.35 constants and the s() curve are not stated by Rocket Science. Applied #2 also points to smish.dev for rotation damping math.

## 8. Sources

### RS-KB (written transcripts)
- https://rocketscience.fyi/know
- https://rocketscience.fyi/know/videos/boost-and-jump
- https://rocketscience.fyi/know/videos/buggy-bounces
- https://rocketscience.fyi/know/videos/dodges
- https://rocketscience.fyi/know/videos/shot-power
- https://rocketscience.fyi/know/rl/issues
- https://rocketscience.fyi/sitemap.xml (checked for other transcripts; only the four above plus camera-settings)

### RS-CC (YouTube auto-captions via yt-dlp, 2026-10-07)
Channel: https://www.youtube.com/@RocketScience (ID UCfKidiMlHTBRNkQZlLzUesw)
- #4 Ball physics: https://youtu.be/9uh8-nBlufM
- #6 Mistakes and additions: https://youtu.be/CPlOh5cAizo
- #7 / #7.1 Deadzone, Dodge deadzone: https://youtu.be/2BHjvhxw06A , https://youtu.be/r1nM0-lYsDc
- #8 Car hitboxes and turning: https://youtu.be/Ymv_ARs33rY
- #10 Wheel hits: https://youtu.be/pTAVP00xwF4
- #11 Supersonic != max speed: https://youtu.be/mlWY6x8g5Ps
- #16 Netcode and lag: https://youtu.be/c373LsgiXBc
- #18 Best turning cars: https://youtu.be/4OBMq9faWzg
- #19 Input consistency and heavy car bug: https://youtu.be/LyzXXO6Z7ZI
- Applied #1 Dribbling: https://youtu.be/sLOxr4V-_r0
- Applied #2 Fast aerials: https://youtu.be/Y9o8ZPEwwK8
- Mini #1 Hitboxes are tilted: https://youtu.be/USBZGpoJa10
- Mini #2 Boostpad hitboxes: https://youtu.be/xgfa-qZyInw
- Heatseeker physics: https://youtu.be/0d_k3EUVRrE
- Patch 1.43/1.44: https://youtu.be/QU46Poqmpnc
- Patch 1.45: https://youtu.be/MXxjtsaT5kY
- Patch 1.56 Plank = Batmobile: https://youtu.be/cDYFM8KDnA4
- The Merc preset explained: https://youtu.be/NmyDRWvFbZ0
- NoFlip/Airdash: https://youtu.be/YLUMcvxLXyg
- Landing wavedash: https://youtu.be/baNsqFEfRMY
- Channel listing (68 videos): /tmp/rocket-science-channel-videos.md

### Psyonix GDC 2018 (Jared Cone)
- Slide deck PDF: https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf
- Glasp summary of the video: https://glasp.co/youtube/ueEmiDM94IE (video: https://www.youtube.com/watch?v=ueEmiDM94IE)
- GDC Vault page: https://www.gdcvault.com/play/1024972/It-IS-Rocket-Science-The

### Third party
- smish.dev ball_simulation_3: https://www.smish.dev/rocket_league/ball_simulation_3/

### Used only to locate sources
- https://gamedeveloper.com/design/video-the-rocket-science-behind-i-rocket-league-s-i-physics
- https://www.rocketleague.com/news/rocket-league-at-gdc-2018
- https://videos.feedspot.com/rocket_league_youtube_channels/
- https://www.huangwm.com/wp/archives/2577 (snippet only)

### Cited by RS, not fetched
- Corey Davis, GDC 2016: https://gdcvault.com/play/1023197/Rocket-League-The-Road-From

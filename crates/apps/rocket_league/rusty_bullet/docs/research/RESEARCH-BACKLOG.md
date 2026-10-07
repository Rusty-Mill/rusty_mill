# Research Backlog

Tracks what's already established (ground truth — don't re-derive) and what
remains genuinely open. Status vocabulary matches the rest of the repo:
`Settled`, `Open`, `Blocked`.

## Settled (ground truth — cite, don't re-research)

### RB-RESEARCH-S001 — Architecture, per Psyonix's own disclosure

- **Source**: Jared Cone (Lead Gameplay Engineer, Psyonix), GDC 2018, "It IS
  Rocket Science! The Physics and Networking of Rocket League."
  [Video](https://www.youtube.com/watch?v=ueEmiDM94IE),
  [slides](https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf).
- **Finding**: Client and server both run the same physics simulation
  (Bullet). Server is authoritative; each client also predicts locally so
  input feels instant. Network traffic is periodic authoritative-snapshot
  corrections, not continuous full state. Visible lag/rubber-banding is
  almost certainly reconciliation-smoothing behavior, not raw network
  latency.
- **Used by**: ADR-0001, RB-NET-001. This is the single best public
  technical source and anchors all physics/netcode research.

### RB-RESEARCH-S002 — Physics engine identity

- **Finding**: Rocket League uses a modified Bullet Physics integration
  inside Unreal Engine 3, running fully client-side (confirmed by offline/
  local play requiring no network connection). Bullet is zlib-licensed
  (permissive, no copyleft) but effectively unmaintained upstream since
  v3.2.4 (April 2022) — still stable/production-trusted (Red Dead
  Redemption 1/2), but not actively evolving upstream.
- **Used by**: ADR-0003, CHARTER.md, SYSTEM-ARCHITECTURE.md.

### RB-RESEARCH-S003 — No public Psyonix Bullet fork exists

- **Finding**: Checked Bullet's fork network and issue tracker for
  Psyonix-attributable activity; nothing surfaced. Their integration is
  almost certainly a private internal fork predating the 2015 release.
- **Implication**: No source code comparison is possible; fidelity
  validation must be behavioral (divergence scoring), never
  code-comparison. Feeds directly into ADR-0003's fidelity-vs-engine-choice
  split.

### RB-RESEARCH-S004 — Replay data gap

- **Finding**: Replay files (parseable via the Rust crate `boxcars`) give
  high-fidelity positions/rotations/velocities/boost state, but do not
  reliably capture raw controller inputs (throttle/steer/jump/boost/
  air-roll) — those are lossy/inferred at best.
- **Confidence basis**: Confirmed against prior first-hand experience
  building a replay viewer/analyzer over ~1,000 replay files.
- **Used by**: RB-VERIFY-001 (non-goals), ADR-0002.
- **Update (2026-08-28, RB-VERIFY-001 implementation)**: More precise than
  originally stated. `subtr-actor` (the crate `rb_replay_ingest` uses on
  top of `boxcars`) recovers real input state directly from the replay's
  replicated vehicle-input actor: raw throttle/steer bytes, and boolean
  flags for boost-active/jump/double-jump/dodge/powerslide — not purely
  inferred from position deltas. Still coarser than live controller
  telemetry (dodge direction comes through as an impulse/torque vector,
  not raw analog stick position, and throttle/steer are single bytes, not
  full per-axis controller resolution), so "lossy/inferred at best" was
  too pessimistic, not wrong in spirit. See `RB-VERIFY-001`'s Non-goals for
  the corrected framing. Wired into `rb_domain::CarState.input` as of
  `RB-VERIFY-001-FR-004` / ADR-0005 (2026-08-28).

### RB-RESEARCH-S005 — BakkesMod is offline-only ground truth now

- **Finding**: Rocket League added Easy Anti-Cheat this year, which now
  blocks BakkesMod during online matches — it's only usable in local/
  offline play against bots. BakkesMod exposes a `ControllerInput` struct
  at high frequency, but only reachable offline.
- **Implication**: Real per-frame controller input can only be captured
  offline, never from live online matches.
- **Used by**: RB-VERIFY-002 (non-goals), ADR-0002, CHARTER.md.

### RB-RESEARCH-S006 — Net verification gap

- **Finding**: No single existing data source gives (real online match) +
  (raw inputs) + (resulting physics state) all at once. Combining
  S004 + S005, the verification pipeline has to be designed around this
  gap explicitly, not assume it away.
- **Used by**: ADR-0002, RB-VERIFY-003 (non-goals), SYSTEM-ARCHITECTURE.md.

## Open (tracked, not yet decided)

### RB-RESEARCH-O001 — Build vs. integrate physics engine (RESOLVED)

- **Question**: Roll a from-scratch physics core (more control, matters
  more for the preservation goal and for tuning against the divergence
  metric) vs. integrate an existing engine (e.g. Rapier — faster path to a
  working netcode testbed, less faithful to Rocket League's actual
  Bullet-based car feel)?
- **Status**: Resolved by [ADR-0004](../adr/0004-bullet3-source-port-for-physics-core.md):
  neither "unguided from scratch" nor "integrate an unrelated engine" — a
  direct, cited Rust port of Bullet3's own (public, zlib-licensed)
  algorithms, implemented as `crates/rb_physics_bullet`. Decided ahead of
  `PHASE-0-EXIT` divergence data existing, on the strength of Bullet3's
  direct relevance and permissive license — see ADR-0004's "Validation and
  revisit triggers" for what would reopen this.
- **Owner**: baileyrd.

### RB-RESEARCH-O002 — Binary reverse engineering as a supplementary source

- **Question**: Should this project reverse-engineer the shipped Rocket
  League client binary to recover client-side physics constants, as a
  supplementary verification source beyond replay/BakkesMod data?
- **Status**: Open — legal review below completed; practical work not
  started and currently blocked (see "Practical blocker"). No prior art
  found doing this specifically (for Rocket League's physics constants).
- **Legal review findings** (2026-08-28, web search against Epic Games'
  and Psyonix's current public terms — not legal advice, and not a
  substitute for the owner's own counsel before acting on it):
  - Psyonix's own EULA page (psyonix.com/eula) redirects to Epic Games'
    Terms of Service/EULA family. Epic's EULAs (Epic Games Store EULA,
    Fortnite EULA, and the general pattern across their agreements)
    contain an explicit contractual clause prohibiting reverse
    engineering, decompiling, or disassembling the software, and deriving
    source code from it.
  - Rocket League's Code of Conduct separately prohibits "exposing
    unreleased features or content found within Rocket League's code,"
    which would plausibly also cover publishing anything recovered via
    binary RE, independent of the EULA's RE clause itself.
  - This is a **contractual** prohibition (breach of the EULA the owner
    agreed to by installing/playing the game), which is a different legal
    question from the U.S. DMCA's §1201(f) interoperability exception
    (which permits some circumvention specifically for achieving
    interoperability of independently created software) — this backlog
    entry does not attempt to resolve how those two interact for this
    specific case; that is exactly the kind of judgment call that needs
    the owner's own legal counsel, not a research-backlog conclusion.
  - Net effect of the legal review: proceeding would likely breach the
    EULA the owner has agreed to, independent of whether it's otherwise
    lawful. That's a real cost (account/access risk at minimum) this
    backlog did not previously make concrete.
- **Practical blocker** (2026-08-28): this project's current working
  environment (a sandboxed cloud dev container) has no Rocket League
  installation and no access to the client binary at all. Any actual RE
  work would have to happen on the owner's own machine, not in this
  session — this document can research the legal question but cannot
  itself perform or stage binary analysis here.
- **Constraints to resolve before proceeding**: given the above, whether
  any output derived from it (constants, documentation) would be safe to
  keep in this repository at all, given the project's stated non-goal of
  using or redistributing any Psyonix code (see SYSTEM-ARCHITECTURE.md
  "Legal and IP boundary"). Extracting a numeric constant is arguably a
  different question from extracting code, but that distinction has not
  been legally evaluated here and shouldn't be assumed favorable.
- **Revisit trigger**: Requires explicit owner sign-off after the owner's
  own review of the above (ideally with actual legal counsel, given the
  EULA breach risk this review surfaced) — not an inferred green light
  from this document or from project momentum. Not blocking Phase 0 or
  Phase 1 start; both have proceeded without it.
- **Owner**: baileyrd.

### RB-RESEARCH-O003 — Scope of BakkesMod offline-capture tooling (RESOLVED)

- **Question**: Does the BakkesMod-side capture tool (RB-VERIFY-002) need
  to be a proper, reusable capture harness (versioned format, configurable
  sampling, robust to BakkesMod API changes) or is a one-off script
  sufficient for Phase 0's needs?
- **Status**: Resolved by [ADR-0005](../adr/0005-capture-file-format-and-input-schema.md):
  a one-off script writing an unversioned JSON-Lines format, not a
  reusable harness — decided per this entry's own stated default, at the
  point `RB-VERIFY-002`'s `rb_capture_ingest` side was actually
  implemented. No re-capturing workflow exists yet to justify more.
- **Owner**: baileyrd.

### RB-RESEARCH-O004 — Larger replay corpus: where to get replay files

- **Why**: the owner's own captures and 40 real matches are the only
  ground truth; more `.replay` files widen coverage (hard landings, wall
  and corner impacts, pogos) and feed any bot-training or detector work.
- **Sources found (2026-10-05, from web search and the ballchasing API
  docs; none downloaded yet)**:
  - ballchasing.com API (needs an API token, never committed; set in the
    environment). Replay list filters: `title`, `player-name`, `player-id`,
    `playlist`, `season`, `match-result`, `min-rank`, `max-rank`, `pro`,
    `uploader`, `group`, `map`, `created-before/after`,
    `replay-date-before/after`, `count`, `sort-by`, `sort-dir`. Raw file:
    `GET /replays/{id}/file`. Detail stats include `flip_resets` (counted by
    the game, unverified by ballchasing), `aerial_hits`, `epic_saves`,
    `clears`, `centers`, `first_touches`, `crossbar_hits`, `bicycle_hits`,
    `juggle_hits`. No mechanic filter on the list endpoint and no pogo stat.
    Download limits: free 1/s and 200/h; patron tiers 2/s and up to 2000/h.
    Group download: free about 333 3v3 games, 1000 per group for the top
    tier. [API](https://ballchasing.com/doc/api),
    [FAQ](https://ballchasing.com/doc/faq).
  - Kaggle: [High-Level Rocket League Replay Dataset](https://www.kaggle.com/datasets/rolvarild/high-level-rocket-league-replay-dataset)
    (about 120k GC+ replays: 41k 1v1, 42k 2v2, 36k 3v3, from ballchasing;
    pipeline at [Rolv-Arild/rl-high-level-dataset](https://github.com/Rolv-Arild/rl-high-level-dataset)),
    and per-season SSL sets such as
    [Season 15](https://www.kaggle.com/datasets/rolvarild/rocket-league-ranked-replays-season-15-ssl)
    (seasons 10-15 listed). Size and licence not checked.
  - Scraper projects showing how to pull replays:
    [RLBot-Dataset](https://github.com/jeromepl/RLBot-Dataset),
    [rlcs_data](https://github.com/Dyl-M/rlcs_data),
    [rocket-league-replays](https://github.com/rocket-league-replays/rocket-league-replays).
  - Local replay folder in-game: `Documents\My Games\Rocket League\TAGame\Demos`.
- **Caveats**: replay frames are sampled at a lower rate than BakkesMod
  captures and carry no raw inputs, so replays check trajectories, not
  one-step physics. Downloaded files go in the gitignored `/replays/`;
  respect each source's terms; check licences before any redistribution.
- **Next**: owner creates a ballchasing token; write a small fetch script
  (outside the workspace crates, or a new adapter) that lists with the
  filters above, keeps `flip_resets > 0`, `aerial_hits`, `juggle_hits`
  candidates, and writes into `/replays/`; run `corpus_check` over them.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O005 — A high-level bot to perform the mechanics

- **Why**: the owner cannot execute some mechanics (pogo, flip reset,
  ceiling shot, speed flip) as accurately as a strong bot; a bot driving
  the game offline while BakkesMod records would give clean, repeatable
  captures for each mechanic, with exact inputs.
- **Candidates found (2026-10-05, web search; not evaluated)**:
  - [RLBot](https://rlbot.org/faq/): framework giving bots game state and
    carrying back button presses for offline modes; Python and .NET.
    [v5 download](https://rlbot.org/v5/).
  - [Necto](https://github.com/Rolv-Arild/Necto) / Nexto: community
    reinforcement-learning bots trained with RLGym; Nexto reported about
    Grand Champion 1 in 1v1/2v2/3v3.
  - RLGym: Gym-style Python API and BakkesMod plugin for training.
  - Not from the search, to verify: RocketSim is the simulator RLGym-sim
    variants train against, which is also this project's reference engine.
- **Open questions**: do existing bots perform the specific mechanics on
  demand (pogo, flip reset) or only as part of play; scripted sequences
  (RLBot) versus a trained policy; how to record BakkesMod captures during
  a bot-driven offline session (the existing capture script and
  `rb_capture_ingest` format, ADR-0005); terms of use of RLBot with the
  current game build; whether a bot should instead drive `rb_physics_bullet`
  directly for self-consistency tests.
- **Next**: the plan is in [BOT-CAPTURE-PLAN.md](BOT-CAPTURE-PLAN.md):
  scripted "tape player" bots plus RLBot state-setting, recorded by the
  existing BakkesMod plugin; first spike a trivial tape on the owner's
  machine. Finding: Necto/Nexto cannot be told to perform a named mechanic.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O006 — Mechanics catalogue to drive the bot and the tests

- **Why**: a bot (O005) and a replay detector (O004) both need a shared,
  named list of mechanics with how each is detected and which physics it
  stresses.
- **Status**: Started: [MECHANICS-CATALOGUE.md](MECHANICS-CATALOGUE.md)
  holds the first list. **Owner**: baileyrd.

### RB-RESEARCH-O007 — Long term: RLBot in Rust, over this physics engine

- **Why**: the owner wants to redo RLBot in Rust as a long-term growth item.
  "Redo RLBot" can mean two different things; they should not be mixed.
- **Reading A, a Rust replacement for RLBot core against the real game.**
  Core (C#) is the v5 server: it talks to the game, launches bots and
  scripts, and speaks FlatBuffers over sockets
  ([core](https://github.com/RLBot/core)). Its game-facing part is a
  native bridge (`Bridge.dll` in `RLBotCS/lib`), and the old v4 repo says
  the source behind `RLBot.exe` is withheld "to meet the legal needs of the
  Psyonix API" ([RLBot](https://github.com/RLBot/RLBot)). A Rust core would
  still need that bridge as a binary, so the rewrite would replace only the
  parts around it, for little gain. Not recommended.
- **Reading B, an RLBot v5-compatible server backed by `rb_physics_bullet`
  instead of the game (recommended direction).** A new adapter crate
  (working name `rb_rlbot_core`) would speak the same FlatBuffers protocol
  ([flatbuffers-schema](https://github.com/RLBot/flatbuffers-schema):
  `GamePacket`, `ControllerState`, `DesiredGameState`, `MatchConfiguration`,
  `FieldInfo`, `BallPrediction`, plus socket framing), build a `GamePacket`
  from `PhysicsFrame`s each tick, step `PhysicsWorld` with the bots'
  controller inputs, and apply `DesiredGameState`. Any v5 bot (the Rust
  `rlbot` crate's examples, Python bots, Nexto-class bots, the scripted tape
  player in `BOT-CAPTURE-PLAN.md`) could then run headless on Linux with no
  game, for regression tests, scenario sweeps and training. It also gives
  the port a second driver beyond recorded inputs.
- **Not in the port today**: boost pads and pickup, goals and scoring,
  kickoff and countdown, match phases and clock, demolitions and bumps, the
  other game modes, ball prediction. Each is a real piece of work; the
  physics ones are tracked as gaps in the mechanics catalogue.
- **Open questions**: the licence terms of the FlatBuffers schema; whether
  the protocol is stable across v5 releases; how much of the packet bots
  actually read (start with players, ball, boost, match info); tick pacing.
- **Phasing (unscheduled)**: (1) finish the capture spike and the tape
  player; (2) read the schema and write a protocol crate (packet types and
  framing only); (3) a minimal server that runs one car and the ball, so a
  `rlbot` crate bot drives `rb_physics_bullet`; (4) boost pads and match
  rules; (5) compare bot runs on the game and on the port.
- **Shared bot surface (added 2026-10-06)**: reading B and the stepping
  environment of ADR-0060 are the same idea at two heights. Order them so
  nothing is built twice: `Env` (reset/step/observe over `PhysicsWorld`,
  ADR-0060) first; then a protocol adapter that turns `Env` observations
  into v5 `GamePacket`s and `ControllerState`s back into `ControllerInput`s.
  For bot code that should run on both targets, the seam is the packet and
  controller types, not a Rust trait of our own: a bot written against the
  `rlbot` crate runs on the game via RLBot core and on the port via the
  adapter, and the tape bot is the first such bot. No new bot trait until a
  second in-process caller needs one.
- **Status**: Open, long term, not scheduled. Needs an ADR before work
  (new adapter crate, new dependencies, public protocol surface).
  **Owner**: baileyrd.

### RB-RESEARCH-O008 — Capture plugin records no inputs for an RLBot-driven car

- **Evidence (2026-10-06)**: in all eleven tape-bot captures
  (`replays/<scenario>.jsonl`, gitignored) every car `input` field is
  zero on every frame, although the car jumps, dodges and boosts as the
  tape says and `rb-verify --scenario ... --against` lines the motion up
  with lag 0 or 1. `RustyBulletCapturePlugin.cpp` reads
  `CarWrapper::GetInput()` after `TAGame.Car_TA.SetVehicleInput`; RLBot's
  input path evidently does not update it. Human captures (`test2.jsonl`)
  do carry inputs.
- **Effect**: `rb-verify`'s "recorded inputs that differ from the tape"
  is meaningless for bot captures (it counts the tape's own non-neutral
  ticks), and `--scenario-from` cannot cut a bot capture into a scenario
  with inputs. The tape is the input ground truth for these runs.
- **Reproduce**: `rb-verify --scenario tools/rb_tape_bot/scenarios/prompt_dodge.json --against replays/prompt_dodge.jsonl`
  reports 23 mismatches; the capture's `input.jump` is never true.
- **Next**: read the input from the vehicle-input event's argument or
  from the PRI/controller rather than `GetInput()`, or record RLBot's
  `last_input` on the bot side; check with a human-driven and a
  bot-driven car in one capture.
- **Status**: Fixed in plugin 1.3 (records the `SetVehicleInput` argument); verified 2026-10-06 on four bot-driven `prompt_dodge` tapes, inputs non-zero and matching the tape (`BOT-RUN-SHEET.md`, session 2 stage 1). **Owner**: baileyrd.

### RB-RESEARCH-O009 — One 5-tick hole per tape-bot capture

- **Evidence (2026-10-06)**: every one of the eleven captures has exactly
  one gap of 0.0417 s (5 missing physics ticks) within the first two
  seconds of the tape (prompt_dodge at tick 52, late_dodge 241,
  wavedash_early 100, wavedash_mid 72, wavedash_late 83, speed_flip 140,
  half_flip 85, pogo 23, hard landing 77, corner_slide 86, car_over_ball
  85), plus 2 to 7 missing ticks at the state-set frame. RLBot core
  itself delivered 120 packets and 120 frames per second without a gap
  over 60 s (`rb_probe`), so the hole is in the plugin's path (the
  per-tick hook or the `GetPhysicsFrame` dedupe) or a game hitch.
- **Effect**: `rb-verify --scenario --against` aligns by timestamp since
  RB-VERIFY-003 0.23.0, so the comparison skips the hole (before that,
  every hole shifted the rest of the comparison: prompt_dodge read 8.5 uu
  mean instead of 1.2). A 5-tick hole during a dodge still hides the ticks
  of most interest, and the 2 to 7 missing ticks at the set frame hid the
  car_over_ball contact itself.
- **Reproduce**: any `replays/<scenario>.jsonl`; list consecutive
  `timestamp_secs` deltas above 0.0125.
- **Next**: log the physics frame number per line and compare with the
  frame RLBot reports; check whether the hole coincides with the
  plugin's first write after `rb_capture_start` or with a fixed interval.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O010 — Car-ball hit: the game's impulse is larger and lasts several ticks

- **Evidence (2026-10-06)**: `car_over_ball` tape-bot capture
  (`replays/car_over_ball.jsonl`, gitignored; scenario
  `tools/rb_tape_bot/scenarios/car_over_ball.json`, the `hitjump.jsonl`
  77.642 s state). Car at 2150 uu/s onto a resting ball. The first
  contact tick agrees: port tick 3 (43.9, 112.0, 163.1) velocity
  (-1211, -1447, 400); game (43.9, 112.1, 163.2), (-1210, -1443, 405).
  Then the game's car keeps gaining upward velocity for four ticks (405,
  444, 475, 498 uu/s) and its horizontal velocity falls to (-1138,
  -1311), while the port's contact ends in one tick (400, 392, 387;
  -1214, -1452). Total game delta-v about (+221, +348, +178) against the
  port's (+145, +208, +77). The ball leaves at 2050 uu/s in the game
  (-862, -1838, -290, then bounces off the floor).
- **Reproduce**: `rb-verify --scenario tools/rb_tape_bot/scenarios/car_over_ball.json --against replays/car_over_ball.jsonl 1`
  (156 uu mean, over 100 uu at tick 71) and `rb-verify --scenario
  tools/rb_tape_bot/scenarios/car_over_ball.json 1` for the port's own
  ticks 0 to 6.
- **Parameter sweep (2026-10-06, rejected, nothing committed)**: the same
  hit is in the human capture (`hitjump.jsonl` 77.642 s: the car gains
  405, 444, 475, 498 uu/s of upward speed over four ticks there too), so it
  is the game's behaviour, not a bot artefact. Windowed objective: k = 8
  prediction over `hitjump` 77.6-77.8 s plus `test2` 5.7-5.9 s and
  12.2-12.4 s (the two guard hits), 58 ms per evaluation. Grid of 648
  combinations over car-ball restitution (0 to 0.2), friction (0.25 to 2;
  no effect at 1 or above), the extra-impulse scale (0.4 to 1.0), its z
  scale (0.2 to 0.5) and forward scale (0.4 to 0.9). The window optimum
  scales the extra impulse by 0.6-0.7 (z scale 0.5): `hitjump` window car
  velocity error 51 -> 8 uu/s, guards' ball error up from 1.9-2.9 to
  2.6-3.7 uu. **The full-capture gate rejects it**: with scale 0.7 and z
  0.5, k = 30 `test2` mean ball distance doubles (0.54 -> 1.17 uu),
  `hitjump` ball 92.95 -> 93.57, and car velocity barely moves (`hitjump`
  10.60 -> 10.46, `test2` 1.99 unchanged). So no contact material or
  extra-impulse setting fixes the multi-tick hit; the window win was one
  event. Lesson for later sweeps: a windowed objective finds candidates,
  the full-capture k = 30 table (test2, hitjump, front, side) decides.
  The cause is structural (what keeps the car and ball in contact for four
  ticks), as the next step says.
- **Partly fixed (2026-10-06, FR-137, ADR-0062)**: the human capture shows
  the hit is one frame of contact: at 77.6667 s the ball (centre 123 uu
  ahead of and 73 below the car's origin, 0.8 uu inside the hitbox) has
  already left at 1700 uu/s; from the next frame it is a gap of 5.5 uu and
  growing, yet the car keeps gaining speed (+95, +40, +30, +23 uu/s) from
  its wheel rays (FR-129) still reaching the ball. The port's ball had
  moved twice as far on the hit tick (16.6 uu against the game's 8.3)
  because the extra hit velocity was added before the position update; it
  now matches (-4.4, -6.2, 93.1 against -5.1, -6.6, 92.1) and the car keeps
  gaining speed after the hit. What remains: the port's push is
  front-loaded (+129 uu/s on the first tick after, then +19, against the
  game's +40, +30, +23), so the ray-ball suspension force is stronger and
  shorter than the game's.
- **Push shape, localised (2026-10-06, nothing committed to the physics)**:
  scratch tracked runs over `hitjump` 77.6-77.72 s (an `Env` snapped to the
  recording each tick, then parts of it left free):
  - *Snap everything every tick* (one-step): the port's car matches the
    game's within 4 uu/s on every tick of the hit (405 / 445 / 475 / 498 /
    493 against 400 / 441 / 472 / 495 / 490). The wheel-ray-ball force is
    right when fed the game's states.
  - *Car snapped, ball left free*: the car's first push after the hit is
    +135 uu/s (535) instead of +40 (445); later ticks are fine. *Ball
    snapped, car free*: +36, and a slow drift (20 uu/s by 77.69 s).
  - The trigger is the ball's height on the first tick after the hit: the
    port's ball is at z 93.14, the game's at 92.14. Lowering the free ball
    by 0.5 uu alone brings the push back to 441 (game 445), so it is a
    threshold (probably a second hitbox-ball contact), not a stiffness.
  - Why the ball is 1 uu high: the game's ball moves (-607, -787, -120)
    uu/s' worth in the hit tick (position change times 120), the port's
    solved ball velocity before the extra hit velocity is (-506, -709, 0).
    The game's contact solve pushes the ball down (-120) and about 17%
    faster horizontally; implied extra velocity in the game (total minus
    that) is (-255, -1052, -170) against the port's formula (-305, -1084,
    -279). So the port's contact solve gives the ball no downward velocity
    where the game's does, and its extra velocity is larger in z; the sum
    nearly agrees, the split does not, and the split decides the next tick.
  - Next: find why the port's sphere-box solve leaves the ball's vertical
    velocity at 0 on the hit tick (the ball is 73 uu below the car origin,
    contact normal about 0.54 forward and 0.84 downward by the recorded
    geometry), and whether the game's z scale applies to the extra velocity
    as modelled.
- **Push shape fixed (2026-10-06, FR-138, ADR-0063)**: the cliff was the
  wheel pushback (`trace < pushback_reach`), Bullet's response against a
  static surface, applied to a wheel ray that hit the ball. With the ball's
  rays given spring and damper only, the free run's car upward speed after
  the hit is 400, 436, 463, 483, 475, 469 uu/s against the game's 405, 445,
  475, 498, 493, 488. The remaining difference is the hit tick's solved ball
  velocity (the port's contact solve leaves the ball no downward velocity on
  its floor; the game's ball drops 1 uu) and the extra velocity's z share;
  a sleeping-ball wake rule was tried (no floor contact on the wake tick) and
  did not change the push, so it is not that.
- **Next**: compare the ball's post-hit velocity between port and game
  (`--scenario` prints only the car; add the ball), then look at the
  contact model: a sustained (multi-tick) car-ball contact with the ball
  pinned against the floor, versus the port's single impulse.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O011 — corner_slide is not repeatable run to run

- **Evidence (2026-10-06)**: the same `corner_slide` tape, start state and
  inputs captured five times: position error against the port 100.3 / 177.6
  (session 1), 19.5 / 33.1 and 100.5 / 177.6, 27.7 / 45.0 and 79.7 / 146.3
  uu (mean / max; two unattended batches, `BOT-RUN-SHEET.md` session 2). The
  cars' speeds differ from tick 12 (1160 vs 1233 uu/s) at the first corner
  contact. Every other scenario repeats to under 4 uu mean.
- **Effect**: the 100 uu "corner climb miss" of session 1 is one draw of a
  distribution the game itself spans (19 to 100 uu mean); the port cannot be
  judged to 100 uu on one capture, and a fit to it could chase noise.
- **Ten more runs (2026-10-06, `replays/batch_20261006-200506`)**: mean
  error against the port 16.6, 19.4, 19.4, 79.7, 80.0, 100.1, 101.0, 106.0,
  106.5 uu, a spread of 89.9 mean / 166.5 max; run against run, the largest
  pairwise distance within the first 200 ticks is 139.5 uu (median 44.6).
  The outcomes cluster: peak height at the corner 288 to 289 uu (three runs,
  two bit-identical), 312 uu (three, bit-identical at tick 120) and 326 to
  331 uu (four), so the game is deterministic for some discrete input, not
  noisy. The delay between the state-set frame and the first recorded
  steer input is 1 to 4 ticks (0.008 to 0.033 s) and varies run to run,
  which fits (the car is at 2112 uu/s and meets the wall within a second)
  but does not explain it alone: runs with the same delay still split (two
  at 0.0167 s gave 288.7 and 326.5 uu).
- **Two experiments (2026-10-06, `batch_20261006-201038`, six runs each,
  scenarios in `tools/rb_tape_bot/experiments/`)**:
  `corner_slide_far` (the car started 0.5 s earlier on its ballistic path,
  60 neutral ticks, then the same steps) is **bit-identical in all six runs**
  (pairwise distance 0.0 uu, peak 272.9 uu, port error 23.0 mean / 41.3 max
  every time). `corner_slide_padded` (original start, 24 neutral ticks before
  the first input) still splits into three outcomes (peak 317, 340 uu;
  pairwise up to 93 uu). So the game is repeatable once the car is not set
  next to the wall, and padding the first input does not help: the
  variation comes from the state set itself (the car is placed about three
  ticks from the wall at 2112 uu/s, and whatever differs between runs in the
  first ticks after a set, such as that tick's step size or sub-tick phase,
  decides the contact), not from input timing. The input-delay hypothesis
  above is rejected.
- **Consequence**: `corner_slide` as shipped is a poor ground truth. Cut the
  corner target from a start with a clear approach (`corner_slide_far` is a
  candidate replacement, repeatable, port 23.0 / 41.3 uu; from this start the
  port peaks at 297.8 uu against the game's 272.9, about 25 uu high, the
  opposite sign to session 1's "port climbs only to 276 against 326", which
  was one draw of the unrepeatable original). Any
  scenario that starts within a few ticks of a contact (car_over_ball and
  the hard landings start in contact or a few ticks from it) deserves the
  same check.
- **Promoted and checked on the others (2026-10-06,
  `batch_20261006-201506`, six runs each)**: `corner_slide_far` is now the
  shipped `corner_slide` (the original is `experiments/corner_slide_original.json`).
  `hard_landing_nose_first` as shipped has the same fault: three outcomes
  (mean error 7.3, 9.9, 11.4 uu; cars up to 54.8 uu apart), while
  `experiments/hard_landing_far.json` (started 0.3 s earlier, spin zeroed,
  fitted so the port reaches the original start state at the original
  tick) is bit-identical in all six runs (port error 4.9 / 11.2 uu).
  `car_over_ball` as shipped is repeatable (cars at most 4.7 uu apart, ball
  0.8 uu), but its far variant (`experiments/car_over_ball_far.json`, 0.25 s
  earlier, fitted the same way) is **not** (two outcomes, 113.5 and 146.7 uu
  mean, cars up to 81 uu apart): its variation is not the state set beside
  a contact, so something else varies in a ball hit during free flight
  that the original start does not exercise. Not investigated.
- **Promoted 2026-10-07**: `hard_landing_far` is now the shipped
  `hard_landing_nose_first` (original: `experiments/hard_landing_original.json`);
  port error 4.9 / 11.2 uu, bit-identical over six runs.
- **Next (superseded)**: find the discrete variable. Candidates: the bot's state set
  and first input landing 1 to 4 ticks apart (move the first tape input,
  or pre-roll the car further from the wall so the contact is not the first
  thing that happens); the physics sub-tick phase at the set. Compare run
  against run, not against the port. `run_batch.ps1` now reports the spread
  over all runs (its first version compared only runs 1 and 2 and called
  this scenario repeatable in a ten-run batch).
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O012 — Wall and ramp climbing: the port climbs about 10% slowly

- **Evidence (2026-10-07)**: recorded-input replay (`rb-verify --scenario ...
  --against ... --recorded-inputs`, `batch_20261006-205659`): `probe_wall_ride`
  9.0 / 41.3 uu mean / max, `probe_wall_ride_slow` 16.0 / 58.6,
  `probe_wall_ride_boost` 19.2 / 66.5 (11.8 on its second capture),
  `corner_slide` 22.3, the end of `probe_turn_boost` and `probe_turn_half_boost`.
  Up to the wall's curved base they agree to under 1 uu; on the flat wall the
  game's car climbs about 10% faster (vz 347 against 316 uu/s) at about 1.5%
  lower total speed, a heading difference of about 1.5 degrees built up in
  the floor-to-wall transition. The arena is RocketSim's own collision
  meshes (FR-117), so it is not the geometry.
- **Tried (scratch, nothing committed)**: runtime overrides on
  `PUSHBACK_ERP`, `STICKY_FORCE_BASE`, `SUSPENSION_STIFFNESS`,
  `WHEELS_DAMPING_*`, `SUSPENSION_SUBTRACTION` and a joint coordinate
  descent over 42 recordings. The best set (ERP 0.05, damping 30 / 32,
  subtraction 1.5, stiffness 450) lowers the mean of all 42 from 5.56 to 4.63
  uu and the wall rides by half, but trades against others: `hard_landing`
  2.6 -> 6.1, `turn_boost` 9.2 -> 15.3, `wavedash_early` 7.3 -> 15.4,
  `wall_ride_45` 0.5 -> 1.9. A lower `STICKY_FORCE_BASE` (0.3) alone helps
  walls and hurts every airborne scenario (`half_flip` 2.4 -> 6.3). Sticky force
  along the car's up axis instead of the contact normal is worse everywhere
  (`brake` 0.3 -> 2.1, `pogo` 2.4 -> 15.7). A parameter family that helps some
  and hurts others means missing structure, not a wrong constant.
- **Not run**: the owner's `test2`/`hitjump`/`front`/`side` k = 30 gate, on
  which `PUSHBACK_ERP` 0.1 was calibrated (FR-122); they are not on the
  machine that did this.
- **Next**: find what differs in the wall transition itself, with a
  recording that isolates it (a car placed on the ramp with no input, at rest
  and rolling) before touching constants. Compare the wheel ray hits and
  suspension lengths tick by tick; the game's vz is jumpier than the port's
  (a 3 to 4 tick pattern), so look at the ray cast against the ramp mesh.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O013 — Powerslide: a long reverse slide loses speed too fast

- **Evidence (2026-10-07)**: `probe_powerslide` (1400 uu/s, handbrake, full
  right steer, throttle) 35.2 / 127.0 uu with the recorded input; the other
  slides agree (`ps_straight` 0.3, `ps_left_slow_release` 1.3, `ps_half_left`
  4.3, `ps_release` 5.1) but `ps_boost` 18.4 and `ps_slow` 12.6 do not.
  Every miss starts at tick 127 to 139, when the car has turned past 90
  degrees from its velocity and slides backwards (forward speed -600 uu/s):
  the port's speed falls faster (646 against 750 uu/s) and its spin stays
  high (4.9 against 4.6 rad/s) then decays slower.
- **Tried**: braking while the handbrake is held (worse, 35 -> 127); a joint
  fit of seven handbrake and grip constants over the seven slide captures
  (handbrake lateral and longitudinal grip, rise and fall rates, powerslide
  steer scale, lateral curve end): no value improves the joint error, each is
  at its optimum. The single-capture wins (steer scale 0.85 -> 25 uu) do not
  hold on the others.
- **Next**: the regime is rare in play (a sustained backwards slide); look
  at what a wheel does when its rolling speed reverses under a held throttle.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O014 — A car landing on the ball with its wheels

- **Evidence (2026-10-07)**: `car_over_ball` with the recorded input: car
  26.6 / 51.6 uu, ball 43.8 / 84.9. Ordinary hits agree to a fraction of a
  uu: `hit_boost` 0.3 car, 0.4 ball; `hit_offset` 0.2 / 0.3; `hit_ground`
  0.3 / 0.3. So the contact model is right; what differs is the wheels
  resting on the ball (the ball is about 1.2 uu lower and 5% faster in the
  game after the first ticks).
- **Tried**: the wheels' suspension impulse reacted on the ball (Newton's
  third law, scratch): any scale makes the car much worse (0.03: car 26.6 ->
  65.8 while the ball 43.8 -> 13.9; 0.5: ball 242), so the game does not
  react on the ball, as Bullet.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O015 — Smaller residuals

- `wavedash_early` 7.3 / 17.4 uu (recorded input): the flip that hops back
  up; chaotic in the landing. `half_flip` 2.4 / 11.8, `pogo` 2.4 / 4.3,
  `turn_fast` 3.0 / 10.9, `turn_slow` 2.7, `hard_landing_nose_first` 2.6 /
  5.4. `corner_slide` (the repeatable start) 22.3 / 40.5 with a port peak
  about 25 uu above the game's.
- **Status**: Open (low priority). **Owner**: baileyrd.

### RB-RESEARCH-O016 — A ball set at rest in the air hovers in the game

- **Evidence (2026-10-07)**: `probe_ball_drop` (ball set at (0, 0, 1500),
  velocity exactly zero): the game's ball stays at z = 1500 with zero
  velocity for the whole 3.5 s tape (the capture shows it), while the port
  drops it (750 uu mean error). A ball with any velocity set (`ball_drop_spin`
  has angular velocity, `ball_wall`, `roof_drop`'s -1 uu/s) moves normally.
  It looks like a sleeping body that nothing wakes until it is touched.
- **Effect**: only state setting can produce it (a training pack's still
  ball); harmless for scoring if scenarios set a nonzero velocity.
- **Status**: Open (low priority). **Owner**: baileyrd.

### RB-RESEARCH-O017 — A car hitting the side ramp from the air bounces differently

- **Evidence (2026-10-07)**: `probe_wall_land` (a car jumping at 850 uu/s
  toward the side wall at 45 degrees, `batch_20261006-212011`), recorded input:
  77.1 / 278.4 uu. Agrees to 0.2 uu until the first contact at tick 78
  (car at x 4007, z 194, in the air, velocity (651, 650, 106)); then the
  game's car goes to (-31, 452, 119) (it bounces off and falls back, x
  falling slowly) and the port's to (79, 492, 205) (it sticks and rides up
  the wall): a 145 uu/s velocity error that stays. The port's contact is
  the real mesh normal (-0.955, 0, 0.297) at (4091.8, ., 196.8), depth
  1.97; the game's change of velocity (-682, -198, +13) looks like a larger
  normal impulse and about 0.4 friction against that same normal, the
  port's (-572, -158, +99) like 0.3.
- **Tried (scratch)**: a uniform car-vs-world restitution and friction
  (`CAR_WORLD_MATERIAL`, 0.3 / 0.3): the best for this contact (0.3 / 0.4)
  gives 30.4 uu but wrecks `corner_slide` (22 -> 78), `hard_landing` (2.6 ->
  45) and `wavedash_early` (7.3 -> 38); 0.3 / 0.3 is the best for all the
  others. So a uniform material is not it; something specific to this
  contact (a hitbox corner against the ramp mesh, Bullet's per-contact
  friction or solver row order) differs.
- `probe_ceiling` (boosting up into the ceiling, 31.5 / 149.1 uu) is the
  same family: car-body contact with a surface from the air.
- **Next**: a recording that isolates one hitbox corner against the ramp
  at a few angles and speeds (a car dropped sideways onto the ramp), and the
  same for the ceiling; compare the per-tick velocity change with the
  port's solver rows.
- **Status**: Open. **Owner**: baileyrd.

### RB-RESEARCH-O018 — Car bumps and demolitions

- **Evidence (2026-10-07)**: 18 two-car recordings (hivemind bot, teammates).
  The game's bump gives the bumped car an extra velocity along the bumper's
  heading: 424 / 632 / 828 / 1011 uu/s at bumper speeds 540 / 804 / 1054 /
  1287 (the line through (0, 5/6), (1400, 1100), (2200, 1530)), 0.2 of the
  speed upward, once per pair per 0.25 s, when the contact is on the bumper's
  nose (64.5 uu ahead of its origin); both cars in a head-on.
- **Done**: `RB-PHYSICS-001-FR-140` (the bump) and `FR-141` (box-box contacts
  as `dBoxBox`), ADR-0068; thirteen scenarios within about 10 uu per car.
- **Demolitions (done)**: `RB-PHYSICS-001-FR-142`. A supersonic (>= 2200 uu/s,
  held to 2100) nose on an enemy removes it at the contact tick (the attacker
  still takes the collision: 2300 -> 1042 uu/s); enemies hit at 1300 and 2100
  are bumped; a teammate at 2300 is bumped. The scenario's `team` makes a
  second car an enemy (its own passive bot).
- **Open**: the respawn (three seconds later, at a spawn point the game picks,
  still, with 0 or 33 boost), a victim in the air, a bumper on a wall or
  ceiling, the bump's cooldown against a second car; bumper speed 2100
  (second car 10 uu) and the off-centre clip (10 / 37 uu max) are the loosest
  fits.
- **Status**: Mostly done. **Owner**: baileyrd.

## Change history

- 2026-10-07: Added RB-RESEARCH-O018 (car bumps and demolitions): bumps
  modelled (FR-140, FR-141), demolitions open.
- 2026-10-07: Added RB-RESEARCH-O017 (car hitting the side ramp from the air;
  `probe_ceiling` in the same family).
- 2026-10-07: Added RB-RESEARCH-O012 (wall and ramp climb), O013 (powerslide
  reverse slide), O014 (car landing on the ball), O015 (smaller residuals),
  O016 (ball hovering when set at rest). RB-RESEARCH-O009's tape-start delay is
  worked around by the recorded-input replay (RB-VERIFY-003-FR-016).
- 2026-10-06: Added RB-RESEARCH-O011 (corner_slide not repeatable);
  RB-RESEARCH-O008 fixed (plugin 1.3, verified). Largest capture hole in the
  22-capture batch 0.075 s (O009 unchanged).

- 2026-10-05: Added RB-RESEARCH-O007 (long term, RLBot in Rust over this
  engine) from the owner's note; reading B recommended, nothing scheduled.
- 2026-10-05: Added RB-RESEARCH-O004 (replay corpus sources),
  O005 (high-level bot) and O006 (mechanics catalogue) from the owner's
  notes; nothing downloaded or built yet.
- 2026-08-28: RB-RESEARCH-O003 resolved (ADR-0005: JSON-Lines capture
  format, one-off script not a harness), decided while implementing
  `rb_capture_ingest`. RB-RESEARCH-S004 updated: replay-recovered input is
  now wired into `rb_domain::CarState.input` (`RB-VERIFY-001-FR-004`).
- 2026-08-28: RB-RESEARCH-O001 resolved (ADR-0004: direct Bullet3 source
  port). RB-RESEARCH-O002's legal review completed (Epic/Psyonix EULA and
  Code of Conduct both prohibit reverse engineering contractually; DMCA
  §1201(f) interoperability exception is a separate, unresolved question
  needing the owner's own counsel) and its practical blocker documented
  (no Rocket League client binary accessible in this environment) — still
  open pending owner sign-off, not advanced further.
- 2026-08-28: Initial backlog created at bootstrap, transcribing prior
  research into settled entries and the three open questions from the
  project handoff.

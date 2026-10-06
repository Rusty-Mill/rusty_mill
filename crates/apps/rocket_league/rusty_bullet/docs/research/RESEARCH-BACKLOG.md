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
- **Status**: Open. **Owner**: baileyrd.

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
- **Next**: compare the ball's post-hit velocity between port and game
  (`--scenario` prints only the car; add the ball), then look at the
  contact model: a sustained (multi-tick) car-ball contact with the ball
  pinned against the floor, versus the port's single impulse.
- **Status**: Open. **Owner**: baileyrd.

## Change history

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

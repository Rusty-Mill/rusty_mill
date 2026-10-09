# Per-player metrics catalog — everything derivable for one player

Status as of 2026-06-20. The **exhaustive** list of information and metrics that
can be derived for a single player from a Rocket League `.replay`, organized by
**derivation layer** (how the datum is obtained, and how much you can trust it).

This is the *universe*, not what happens to appear in any one match. Layer A is
grounded in the **complete boxcars attribute table** (`src/data.rs::ATTRIBUTES`,
**298** object→attribute entries — every datum the format *can* replicate, even
if a given replay never exercises it), enumerated in full as the data dictionary
in [`replay-file-format.md` Appendix A](replay-file-format.md#appendix-a--complete-attribute-reference),
plus the header. Layers B–F are what this workspace (and ballchasing/carball)
compute on top. See [`replay-file-format.md`](replay-file-format.md) for the
container and [`ballchasing-analyzer-teardown.md`] *(private repo)*
for the stat math.

Trust tags: **[A]** authoritative (read straight from the bytes — no inference),
**[D]** derived (computed/inferred from kinematics — a model, not a fact),
**[IP]** this workspace's own analytic (heuristic, versioned).

> Caveat that governs every **[D]** item: replays record **motion, not inputs** at
> the analysis layer (the raw input *bytes* in Layer A5 exist but are coarse), so
> anything reconstructed from kinematics is an inference with a confidence, not
> ground truth.

---

## Layer A — Authoritative (replicated in the file)

Everything here is read directly. Per-frame items are sampled at the record rate
(~30 Hz); the rest are header facts or sparse replicated events.

### A1 · Identity & account
- Display name (`PlayerName`); **online unique id** (`UniqueId` → platform +
  platform id + local id) and `PlayerID`; `EpicPUID`; `RemoteUserData`. **[A]**
- Team (0 blue / 1 orange), `PawnType`, `bBot` (is an AI bot). **[A]**
- Club membership (`ClubID`), party leader (`PartyLeader`), split-screen flag
  (`bIsInSplitScreen`), spectator/observer flags. **[A]**
- Anonymization (`AnonymizedName`, `bAnonymizeToOpponents/Teammates`), MMR/skill
  tier hint (`SkillTier`). **[A]**

### A2 · In-game scoreboard & counters (authoritative truth)
From the header `PlayerStats[]` row and the `TAGame.PRI_TA` counters — these are
the game's own tallies, not recomputed:
- `Score` / `MatchScore`, `Goals`/`MatchGoals`, `Assists`/`MatchAssists`,
  `Saves`/`MatchSaves`, `Shots`/`MatchShots`. **[A]** — the network `Match*`
  counters are **replicated incrementally** (each unit lands on its own frame), so
  their rising edges give per-event *timing* for shots/saves/assists, not just
  totals; this is what feeds the Game Timeline (`Event::Stat`).
- `MatchMVP` (`bMatchMVP`), match `GameWinner`/`MatchWinner`/`MVP` (game-event). **[A]**
- Demolition counters: `CarDemolitions`, `MatchDemolishes`, `SelfDemolitions`. **[A]**
- Possession counters: `KeepUpPossessions`, `PossessionClears`,
  `PossessionDenials`, `PossessionSteals`. **[A]**
- Mode damage: `MatchBreakoutDamage` (Dropshot). **[A]**
- Career/session: `TotalXP`, `TotalGameTimePlayed`, `TotalIdleTime`. **[A]**
- Cosmetic titles: `PrimaryTitle`, `SecondaryTitle`, `Title`, `RepStatTitles`
  (the stat-title medals shown on the scoreboard). **[A]**

### A3 · In-game stat-event feed (`ReplicatedStatEvent`)
A sparse, **timestamped, player-attributed** stream of the game's own awarded
events — the medal/stat feed. The event *names* are a known enum; the common
per-player ones: **Goal** (and variants: Aerial, Backwards/Bicycle, Long,
Overtime, Turtle, Pool-shot), **Assist**, **Playmaker**, **Shot**, **Save**,
**EpicSave**, **Savior**, **Center**, **Clear**, **FirstTouch**, **Demolish**,
**HatTrick**, **MVP**, **Win**, plus mode events (Dropshot damage, Rumble item
use, juggles). **[A]** — game-truth events, distinct from the kinematic
re-derivations in Layer C.

### A4 · Per-frame physical state (`ReplicatedRBState`)
The rigid body, every frame the car is alive:
- **Position** `(x, y, z)` in uu. **[A]**
- **Orientation** (wire quaternion → pitch/yaw/roll). **[A]**
- **Linear velocity** `(vx, vy, vz)` uu/s and **angular velocity** (spin). **[A]**
- `sleeping`/`bFrozen` (no-velocity / frozen), `TeleportCounter`,
  `ReplicatedCarScale` (size mutators). **[A]**

### A5 · Per-frame inputs & control state
The actual replicated control surface (coarse but real):
- **Throttle** (`ReplicatedThrottle`, byte), **steer** (`ReplicatedSteer`, byte). **[A]**
- **Handbrake / powerslide** on-off (`bReplicatedHandbrake`). **[A]**
- **Driving** vs airborne-with-no-wheels (`bDriving`), `InputRestriction`
  (countdown/cinematic lockout). **[A]**
- `RemoteViewPitch` (where the player is aiming the car/look). **[A]**
- Per-control sensitivity (`SteeringSensitivity`). **[A]**

### A6 · Boost system
- **Boost gauge** 0–255 every frame (`ReplicatedBoostAmount` / `ReplicatedBoost`). **[A]**
- **Boosting on/off** and for how long, per activation
  (`CarComponent_TA:ReplicatedActive` / `ReplicatedActivityTime` on the boost
  component). **[A]**
- **Pad pickups** — authoritative collection events (`VehiclePickup_TA`,
  instigator = this player) → which pad, big/small, when. **[A]**
- Mutator/economy modifiers when present: `BoostModifier`, `RechargeDelay`,
  `RechargeRate`, `bNoBoost`/`bUnlimitedBoost`, `BoostRestriction`. **[A]**

### A7 · Jump / dodge / aerial mechanics
Each car action is its own component with an active flag + impulse:
- **Jump** and **double jump** (`CarComponent_DoubleJump_TA:DoubleJumpImpulse`). **[A]**
- **Dodge / flip** direction & power (`Dodge_TA:DodgeImpulse`/`DodgeTorque`,
  `Dodge_KO_TA:DodgeRotationCompressed`). **[A]**
- **Flip-reset / recovery** (`FlipCar_TA:FlipCarTime`, `bFlipRight`). **[A]**
- **Air actions remaining** (`AirActivate_TA:AirActivateCount`),
  `DodgesRefreshedCounter` (flip resets granted). **[A]**
- Each component's `ReplicatedActive` + `ReplicatedActivityTime` ⇒ exactly *when*
  and *how long* each mechanic fired. **[A]**

### A8 · Demolitions
- `ReplicatedDemolish` / `…Extended` — **attacker, victim, position, velocity**
  of each demo this player inflicts or takes; `…GoalExplosion` (fx), self-demos. **[A]**

### A9 · Camera & view settings
- **Camera profile** (`CameraSettings`/`ProfileSettings` → FOV, distance, height,
  angle/pitch, stiffness, swivel speed, transition speed). **[A]** — decoded onto
  `PlayerMeta.camera` (+ `steering_sensitivity`); matches ballchasing's `camera`
  object 25/26 players (the miss never replicates `ProfileSettings`).
- **Ball-cam toggle over time** (`bUsingSecondaryCamera`), behind-view
  (`bUsingBehindView`), swivel/free-look (`bUsingSwivel`), mouse toggle. **[A]**
- **Live look direction** (`CameraPitch`, `CameraYaw`). **[A]**

### A10 · Loadout & cosmetics
- Full **loadout** (`ClientLoadout(s)`/`…Online`): car body, decal/skin, wheels,
  boost, trail, goal explosion, antenna, topper, engine audio, banner,
  primary/accent paint colors, paint finishes; product attributes
  (painted/certified/special-edition). **[A]** — the **car body** id is decoded
  and named via `crate::cars` (ballchasing's `car_id`/`car_name`); the cosmetics
  are available in the loadout but not surfaced.
- **Team paint** (`TeamPaint`: team, primary/accent color, finish), club colors. **[A]**

### A11 · Mode-specific per-player items
Present only in the relevant playlist, but part of the format universe:
- **Rumble**: power-up held/active (`RumblePickups`, `AttachedPickup`, the
  `SpecialPickup_*` actors — Boot, Disruptor, Freezer, Grappling Hook, Haymaker,
  Magnetizer, Plunger, Power Hitter, Spike, Swapper, Tornado, Ball Velcro/Freeze,
  Targeted). **[A]**
- **Dropshot**: tile damage state (`BreakOutActor_Platform_TA:DamageState`,
  `MatchBreakoutDamage`). **[A]**
- **Knockout**: state (`Car_KnockOut_TA:ReplicatedStateName/StateChanged`). **[A]**
- **Heatseeker/juggle**: ball keep-up state. **Infection**: `ViralItemActor`
  infected status. **[A]**

### A12 · Connection / session quality
- **Ping** every update (`Ping`), worst net quality (`…WorstNetQualityBeyondLatency`),
  timeout/disconnect (`bTimedOut`). **[A]**
- Lifecycle: join/leave times (PRI actor spawn/destroy), `bWaitingPlayer`,
  `bReadyToPlay`/`bReady`, idle time (`TotalIdleTime`, `bIdleBanned`),
  substitution (a player replacing another). **[A]**

---

## Layer B — Derived kinematic aggregates (ballchasing-class)

Time-uniform reductions of A4/A5/A6 over the resampled grid — the
`analyze::bcstats` block, parity-validated against ballchasing. All **[D]**.

**Movement** — `avg_speed`, `total_distance`, time & % at **slow / boost /
supersonic** speed, time **on-ground / low-air / high-air**, `time_powerslide`,
`count_powerslide`, `avg_powerslide_duration`, `avg_speed_percentage`.

**Boost economy** — `avg_amount`, `BPM` (collected/min), `BCPM` (used/min),
`amount_collected` & `amount_stolen` (× big/small), `count_collected/stolen_*`,
`amount_overfill` (+stolen), `amount_used_while_supersonic`, `time/percent_zero_boost`,
`time/percent_full_boost`, the **0–25/25–50/50–75/75–100 quartile histogram**.

**Positioning** (in the team's attack frame) — `avg_distance_to_ball`
(+possession / no-possession split), `avg_distance_to_mates`, time & % in
**defensive/neutral/offensive third**, **defensive/offensive half**,
**behind/in-front of ball**, **most-back/most-forward** (last-defender share),
`goals_against_while_last_defender`, **time-in-own-half (ball side)**,
`time_closest/farthest_to_ball`.

**Further [D] derivables from A4/A5 not yet in `bcstats`** — acceleration & jerk,
heading-change rate / turning radius, **time on wall / on ceiling**, airborne
height profile, **flip/dodge/double-jump counts** and **wasted-flip** rate (from
A7), facing-ball angle, **boost-starved time at 0** in own half, distance-from-own-
goal profile, demo-avoidance (juke) rate.

---

## Layer C — Derived events & touch analytics

Inferred from ball-velocity discontinuities + nearest car + geometry
(`analyze::events`, and the carball/ballchasing-style recomputations). All **[D]**.

- **Touches** — count, locations (touch heatmap), height, and type (ground /
  aerial / wall / ceiling). **Possession** runs and **possession time**. **First
  touch** of each play.
- **Kickoffs** — role (go / cheat / back / wing), approach, first-touch reaction
  time, kickoff **win/loss/neutral** outcome.
- **Recomputed contributions** — shots, **shots against**, saves, clears, passes,
  **assists/pre-assists**, centers, dribbles, **shooting %** (these are the
  ballchasing "recomputed" stats — advisory vs the A2/A3 game-truth versions).
- **Duels** — 50/50 challenges won/lost/neutral, double-commits, fakes.
- **Demos** with context (A8 joined to positions): demos-for/against, who/where/when.
- **Goals** with context — own goals, open-net vs contested, assist chains.

---

## Layer D — Mechanical skill detections (`skills` crate) · [IP]

A 12-skill catalog detected purely from the canonical grid/events, each with a
verification API (`performed` / `…_by` / `count_for` / `…_in_window`):

**aerial · air dribble · ceiling play · wall play · ground dribble · flick ·
power shot · redirect · kickoff first touch · boost steal · demo · supersonic.**

Per-player: did they perform skill X, how many times, when, and (verify-mode) was
a claimed skill actually performed in a time window. Thresholds are versioned and
corpus-calibrated. *Mechanics, not positioning.*

---

## Layer E — Decision-discipline / positioning scoring (`scoring` crate) · [IP]

The "did they play the position well" rubric, config-driven:

- **Roles** per moment — 1st / 2nd / 3rd man, last-defender.
- **Rotation & positioning metrics** — ~14 sub-metrics (spacing, goalside
  discipline, cheating-up, overcommit, recovery, boost-starve, etc.).
- **Composite score + sub-scores**, a **licence band** (skill-bracket estimate),
  and **"leaks"** mapped to coaching chapters. Calibrated against a ranked corpus
  (composite tracks rank, CV Spearman ≈ 0.79).

---

## Layer F — Value / impact modeling (`value` crate) · [IP]

An independent, calibrated value model used as a second track:

- **Per-touch ΔV** — how much each touch changed the team's scoring-value /
  win-probability (value added / lost per action).
- **Momentum / win-probability contribution** over the match.
- **Goals-above-expected**-style impact, and **two-track reconciliation** (does a
  player's positioning score agree with their value-model impact?).

---

## Not in the replay (must be sourced externally)

Frequently assumed to be derivable per player, but **absent from the file**:

- **Competitive rank / division / MMR.** *Not present.* The only rank-adjacent
  attribute, `TAGame.PRI_TA:SkillTier`, is declared but **unpopulated** in
  practice (both ranked samples emit zero `SkillTier` updates), and there is no
  rank on the `PlayerStats[]` header row. Rank must come from the Rocket League /
  tracker APIs or the uploader's account — which is how ballchasing shows it and
  how this repo's corpus gets per-player `ranks` (in `manifest.json`, fetched from
  ballchasing's API, **not** parsed from the replay). See
  [`replay-file-format.md` §9](replay-file-format.md#9-notable-omissions--what-the-format-does-not-carry).
- **Discrete button inputs** (jump / boost / air-roll presses) — inferred from
  component-activation attributes + kinematics, not logged.
- **Chat / voice content**, and the **real names of anonymized players**.

## How to read this for one player

A complete per-player picture stacks the layers:
**who they are** (A1, A10–A12) → **what the game credited them** (A2–A3) →
**what their car physically did, frame by frame** (A4–A9) → **the ballchasing-
class aggregates** (B) → **what plays they were part of** (C) → **which mechanics
they hit** (D) → **how disciplined their positioning was** (E) → **how much they
actually changed the outcome** (F).

Layers A–C are the *objective/standard* surface (A authoritative, B/C the
ballchasing-class derivations); D–F are this workspace's heuristic IP and should
be read as versioned inferences, not facts.

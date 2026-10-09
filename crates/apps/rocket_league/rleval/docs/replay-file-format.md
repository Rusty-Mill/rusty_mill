# Rocket League `.replay` file format — specification

Status as of 2026-06-20. A formal reference for the binary `.replay` container
this workspace decodes, and the obligations a conformant decoder must meet to
produce our [canonical match model](../../crates/replay-analyzer/src/model.rs). It is the format-layer
companion to [`docs/ballchasing-analyzer-teardown.md`] *(private repo)*
(which is about *stats*) and the contract behind the [decode port](../../crates/replay-analyzer/src/decode/mod.rs).

> **Provenance & confidence.** Psyonix publishes no official format. This spec is
> the community-reverse-engineered structure as implemented by the
> interoperable open-source parsers — **[boxcars]** (our decoder),
> **[rattletrap]** (ballchasing's lineage), and **jjbott/RocketLeagueReplayParser**
> — cross-checked against our own decode of the committed samples. Field *layout*
> and *semantics* below are reliable; the exhaustive **bit-level** encodings of
> compressed vectors/quaternions are version-dependent and deep, so for those we
> specify the structure and defer the last bit to the reference parsers. Tags:
> **[stable]** (format-invariant), **[versioned]** (depends on the version
> triple), **[ours]** (a convention of this workspace, not the file).

Worked example throughout: `assets/replays/42f2.replay` (engine **868**,
licensee **27**, net **10**; `header_size=5492`, `content_size=1314385`;
387 objects, 567 names, 44 net-cache classes, 42 keyframes, **11 642** network
frames).

[boxcars]: https://github.com/nickbabcock/boxcars
[rattletrap]: https://github.com/tfausak/rattletrap

---

## 1. Container layout

A replay is **two length-and-CRC-prefixed sections** — a small *header* and a
large *content* (body) — laid out back to back. All integers are
**little-endian**; this framing is **[stable]** across every version.

```
┌──────────── header section ────────────┐┌──────────── content section ───────────┐
 header_size : u32   (bytes after crc)      content_size : u32  (bytes after crc)
 header_crc  : u32                          content_crc  : u32
 header_data : [u8; header_size]            content_data : [u8; content_size]
```

- **`*_size`** counts the bytes of that section's *data* (after its CRC word).
- **`*_crc`** is Psyonix's custom CRC-32 over the section data (parameters, per
  boxcars: width 32, **poly `0x04C11DB7`**, **non-reflected** in/out, XorIn
  `0x10340DFE`, XorOut `0xFFFFFFFF`). boxcars validates it under
  `.on_error_check_crc()`; a mismatch means a truncated/corrupt file. **[stable]**

A decoder that only needs scoreboard facts can parse the **header alone** and
never touch the (far larger, bit-packed) content. Everything spatial lives in the
content's network stream (§4).

---

## 2. Version triple

The header data begins with the version, which gates several later encodings:

| Field | Type | Example (42f2) |
|---|---|---|
| `engine_version` (major) | u32 | 868 |
| `licensee_version` (minor) | u32 | 27 |
| `net_version` (patch) | u32 | 10 |

`net_version` is **present only when** `engine_version > 865 && licensee_version
> 17`; older replays omit it (read as 0). **[versioned]** Then a length-prefixed
string `game_type` names the replay class, e.g. **`TAGame.Replay_Soccar_TA`**
(standard Soccar). The version triple selects: whether a third version int
exists, the property-id bit width and ids in the net cache, and the
vector/quaternion bit encodings in the network stream (§4.4).

---

## 3. The header section — properties

After the version triple, the header is a single **property map**: a sequence of
`(key, value)` entries terminated by the key **`"None"`**. **[stable]**

Each non-`None` entry is:

```
key        : String                 (length-prefixed; see §6 for string encoding)
kind       : String                 ("IntProperty", "StrProperty", "FloatProperty",
                                      "NameProperty", "BoolProperty", "ByteProperty",
                                      "QWordProperty", "ArrayProperty", "StructProperty")
value_size : u64                     (byte length of the value payload)
value      : depends on `kind`
```

Value encodings by kind:
- **Int** `i32`, **QWord** `u64`, **Float** `f32`, **Bool** `u8` (0/1),
  **Str/Name** length-prefixed string.
- **Byte** an *enum* pair `(enum_name: String, enum_value: String)` (not a raw
  byte) — e.g. camera/platform enums.
- **Array** `count: u32` followed by `count` **nested property maps** (each itself
  `None`-terminated). This is how `PlayerStats[]` and `Goals[]` are carried.
- **Struct** a named struct followed by a nested property map.

### 3.1 Properties this workspace consumes

The decoder reads these by key (see `src/decode/boxcars_adapter.rs::extract_meta`):

| Key | Kind | Meaning |
|---|---|---|
| `MapName` | Name | arena id, e.g. `Stadium_P` (→ `classify_map`) |
| `TeamSize` | Int | players per team |
| `Team0Score` / `Team1Score` | Int | final score (blue / orange) |
| `RecordFPS` | Float | nominal record rate (~30) |
| `NumFrames` | Int | network-stream frame count |
| `MaxChannels` | Int | actor-channel cap (default 1023) — the **actor-id bit width** for §4 |
| `PlayerStats` | Array | per-player scoreboard rows (below) |
| `Goals` | Array | per-goal rows (below) |

A `PlayerStats[]` row carries `Name` (Str), `Team` (Int, 0/1), `Score`, `Goals`,
`Assists`, `Saves`, `Shots` (Int), plus account identity `OnlineID` (QWord) +
`Platform` (Byte enum) and `bBot` (Bool). A `Goals[]` row carries `frame` (Int —
the **network-frame index** of the goal), `PlayerName` (Str), `PlayerTeam` (Int).
These are **authoritative end-of-match truth**, independent of the network
reconstruction. **[stable]**

The **complete header property set** (all keys, plus the `PlayerStats`/`Goals`/
`HighLights` row schemas) is catalogued in
[**Appendix B**](#appendix-b--header-property-reference).

---

## 4. The content section — body + network stream

The content data is a sequence of length-prefixed tables, then the bit-packed
network stream, then trailing tables. Order is **[stable]**:

```
levels        : Vec<String>                      // map packages (usually 1)
keyframes     : Vec<{ time:f32, frame:u32, file_position:u32 }>   // seek index
network_size  : u32
network_data  : [u8; network_size]               // the bit-packed frames (§4.2)
debug_strings : Vec<{ frame:i32, user:String, text:String }>
tick_marks    : Vec<{ description:String, frame:i32 }>            // highlight seeks
packages      : Vec<String>
objects       : Vec<String>                      // ObjectId → name  (§4.1)
names         : Vec<String>                      // NameId   → name
class_indices : Vec<{ class:String, index:i32 }>
net_cache     : Vec<ClassNetCache>               // property dispatch (§4.1)
```

### 4.1 The object / name / net-cache tables (the decode key)

The network stream never carries strings — it references the **`objects`** table
by integer **`ObjectId`**. `objects[ObjectId]` resolves to a dotted name such as
`TAGame.RBActor_TA:ReplicatedRBState` or `Archetypes.Ball.Ball_Default`. The
first entries are engine intrinsics (`Core.Object`, `Engine.Actor:RelativeLocation`,
…); class/archetype and attribute-property names follow. **[stable]**

- **Spawns** reference an **archetype** object id (e.g.
  `Archetypes.Ball.Ball_Anniversary`, a `…:Pawn` car) — that's how an actor's
  *class* is known. The full archetype → class catalog (and which classes spawn
  with a position/rotation) is [**Appendix C**](#appendix-c--archetype--class-reference).
- **Attribute updates** reference a **property** object id (e.g.
  `Engine.Pawn:PlayerReplicationInfo`); the **`net_cache`** maps each class to the
  set of `(property ObjectId, stream_id)` pairs it can replicate. The decoder
  reads a compact `stream_id` off the wire and looks up which property — and thus
  which **attribute type** — to decode. A `ClassNetCache` is
  `{ object_index, parent_id, cache_id, properties: [{ object_index, stream_id }] }`,
  forming an inheritance chain so a subclass inherits its parents' properties.
  **[versioned]** (the id space shifts with the version triple).

### 4.2 The network stream — frames

`network_data` is a single **LSB-first bit stream** (not byte-aligned) of
`NumFrames` frames. **[versioned]** Each frame:

```
time   : f32                      // seconds since match start (32 bits)
delta  : f32                      // seconds since previous frame
actors : loop { has_actor:bit ; if 0 break ; <replicated actor> }
```

A *replicated actor* entry:

```
actor_id  : serialized-int in [0, MaxChannels)        // bounded by the header cap
channel_open : bit
  if !channel_open  → actor is DELETED (closed this frame)
  if  channel_open  →
        is_new : bit
        if  is_new → NEW ACTOR  (spawn)
        if !is_new → UPDATED ACTOR (attributes)
```

- **New actor (spawn):** an optional `name_id` (into `names`, version-gated), a
  flag bit, an `object_id` (the archetype → class), and an **initial trajectory**:
  a compressed `location` Vector and, for classes that rotate, a compressed
  `rotation`. (42f2's first spawn: `actor_id=0`, `object_id=64` →
  `Archetypes.Ball.Ball_Anniversary`.)
- **Updated actor:** `loop { has_prop:bit ; if 0 break ; stream_id ; <attribute> }`.
  `stream_id` is a bounded int resolved through the actor's class `net_cache` to a
  property → **attribute type** → the value decoder (§4.3).
- **Deleted actor:** the channel closes; the actor id may later be **recycled** by
  a different actor (§5).

A frame thus arrives as `{ time, delta, new_actors[], updated_actors[],
deleted_actors[] }` — exactly the shape boxcars exposes and our
`decode::RawFrame` mirrors. **[ours]** for the `RawFrame` naming.

### 4.3 Attribute types

Each replicated property decodes to one **attribute** of a fixed type (boxcars'
`Attribute` enum, ~44 variants). The **complete universe** of object → attribute
mappings the format can carry (298 in boxcars' table) is the data dictionary in
[**Appendix A**](#appendix-a--complete-attribute-reference); the subset this
workspace reads — and the object string that selects each — is the reconstruction
contract:

| Object string (property) | Attribute | Carries |
|---|---|---|
| `TAGame.RBActor_TA:ReplicatedRBState` | `RigidBody` | `location` (Vector), `rotation` (Quaternion), `linear_velocity?`, `angular_velocity?`, `sleeping` flag |
| `Engine.Pawn:PlayerReplicationInfo` | `ActiveActor` | car → PRI link (player identity) |
| `Engine.PlayerReplicationInfo:PlayerName` | `String` | player name |
| `Engine.PlayerReplicationInfo:Team` | `ActiveActor` | PRI → team-actor link (0/1) |
| `TAGame.CarComponent_TA:Vehicle` | `ActiveActor` | boost component → car |
| `TAGame.CarComponent_Boost_TA:ReplicatedBoost{,Amount}` | `ReplicatedBoost` / `Byte` | boost gauge byte (0–255) |
| `TAGame.VehiclePickup_TA:(New)ReplicatedPickupData` | `Pickup` / `PickupNew` | `instigator?` (collecting car) + picked-up flag |
| `TAGame.Vehicle_TA:bReplicatedHandbrake` | `Boolean` | powerslide on/off |
| `TAGame.Car_TA:ReplicatedDemolish{,Extended}` | `Demolish` | attacker car + victim car |

Other notable attribute types in the wire (not all consumed): `Byte`, `Int`,
`Float`, `Enum`, `Loadout`, `CamSettings`, `Reservation`, `UniqueId`,
`PartyLeader`, `StatEvent`, `Location`, `Rotation`, `Pickup`/`PickupNew`. **[versioned]**

### 4.4 Bit encodings (structure; cite the parsers for exact bits)

- **Serialized int (bounded):** actor ids and `stream_id`s are read with a
  "max-bounded" int read — bits are consumed while the running value's next power
  stays under the bound. The bound is `MaxChannels` for actor ids; the class's max
  property id for `stream_id`. **[versioned]**
- **Vector:** a 5-bit `num_bits` prefix, then three components of `num_bits + 2`
  bits each, de-biased and **scaled** to unreal units. boxcars yields positions
  already in **uu** (raw fixed-point ÷ 100). **[versioned]**
- **Quaternion / rotation:** newer nets encode rotation as a compressed quaternion
  (a 2-bit "largest component" selector + three compressed words); older nets use
  per-axis byte Eulers. Our adapter converts whatever boxcars yields to Euler
  `[pitch, yaw, roll]` radians (`quat_to_euler`). **[versioned]**

For the exhaustive bit math (bias constants, word widths per net version) treat
[boxcars] and [rattletrap] as the normative reference — re-deriving it here would
duplicate thousands of lines of version-conditional decoding.

---

## 5. Reconstruction obligations

The network stream is delta/keyframe encoded and identity-hostile. A conformant
decoder targeting our canonical model **must** handle all of the following — these
are the contract `src/analyze/reconstruct.rs` fulfils and `recon-check`
independently verifies. **[ours]**

1. **Carry-forward.** An actor re-reports an attribute only when it changes; a
   stationary ("`sleeping`") rigid body emits no velocity. Hold last-known state
   per actor and treat sleeping as zero velocity.
2. **Actor-id recycling → stable identity.** Car actor ids are destroyed and
   reused on every respawn/demo/goal-reset, so one player fragments into many
   short-lived car actors. Coalesce them onto the stable **PRI** (via the car→PRI
   `ActiveActor` link) into one continuous track, and mark **gaps** (dead/
   respawning intervals) so consumers never interpolate a car *through* its own
   death.
3. **Frame-rate normalization.** Recorded `delta`s are uneven. Resample to a fixed
   grid (~30 Hz) so every `time_*` integral is a clean `count × Δt`.
4. **Gameplay-frame gating.** Post-goal/replay/kickoff-countdown frames are **not
   gameplay** (ball reset, cars frozen/teleported). Exclude `[goal, next-kickoff)`
   windows from possession/positioning/ball-side aggregates.
5. **Multi-ball / decoy robustness.** The ball actor id recycles across goals
   (several moving ids); some non-Soccar modes spawn a stationary secondary
   `Ball`-named actor. Keep only ball actors that actually move.
6. **Team binding at source.** Resolve a PRI's team from
   `Engine.PlayerReplicationInfo:Team` → the team actor's archetype
   (`Archetypes.Teams.Team0/1`), not the fragile header name→team join.

---

## 6. Conventions & units

- **Strings** are length-prefixed: a length `i32`, then the bytes. A **positive**
  length is Windows-1252/ASCII (1 byte/char); a **negative** length is UTF-16LE
  (`-len` chars, 2 bytes each). Both include a trailing null in the count.
  **[stable]**
- **Coordinates** are **unreal units (uu)** after boxcars' ÷100 scaling. Arena
  constants (`src/field.rs`): side walls `x = ±4096`, back walls `y = ±5120`,
  ceiling `z = 2044`, ball radius `92.75`, goal depth `880`. **[stable]**
- **Velocities** are uu/s; supersonic ≥ 2200, max ground speed ~2300.
- **Boost** is a replicated **byte 0–255**; percent = `byte / 255 × 100`. **[stable]**
- **Rotation** in our model is Euler `[pitch, yaw, roll]` radians (converted from
  the wire quaternion). **[ours]**
- **Teams:** 0 = blue, 1 = orange. **[stable]**

---

## 7. Mapping to this workspace

| Format element | Decode port (`src/decode`) | Canonical model (`src/model.rs`) |
|---|---|---|
| header properties | `ReplayMeta` (`extract_meta`) | `map`, `team_size`, `team_scores`, `record_fps`, `players` |
| `PlayerStats[]` row | `PlayerMeta` | `CanonicalMatch::players` |
| `Goals[]` row | `GoalInfo` | `Event::Goal` |
| network frame | `RawFrame` | `FrameOut` / `GridFrame` (after resample) |
| spawn / archetype | `NewActorEvent { class }` | (drives ball/car classification) |
| `ReplicatedRBState` | `ActorUpdate::RigidBody` | `CarState` / `Kin` (p, v, rot) |
| car↔PRI / PRI name / team | `CarPri` / `PriName` / `PriTeam` | `PlayerTrack` (coalesced) |
| boost byte | `ActorUpdate::BoostAmount` | `TrackSample::boost` / `GridCar::boost` |
| pickup event | `ActorUpdate::PickupBoost` | `PadPickupEvent` (`CanonicalMatch::pickups`) |
| handbrake bool | `ActorUpdate::Handbrake` | `PowerslideInterval` (`CanonicalMatch::powerslides`) |
| demolish | `ActorUpdate::Demolish` | `Event::Demo` |

The port deliberately surfaces **only** the attributes reconstruction consumes;
everything else in the stream is skipped. Swapping decoders (e.g. a second parser
for redundancy) means writing another adapter behind the `ReplayParser` trait —
the analyze layer is unaffected. See `recon-check/` for an independent
reconstruction cross-check.

---

## 8. Versioning & forward-compatibility

- The **version triple** is the compatibility switch; a new Rocket League build
  can bump it and change net encodings (notably after **EAC**/anti-cheat
  releases, which periodically perturb the network format). A decoder pins a known
  parser version (`parser_version = "boxcars-<x.y.z>"` is recorded on every
  canonical model) for reproducibility.
- **Unknown attributes / new properties** must be skippable: because the
  `net_cache` declares each class's property set with bounded `stream_id`s, an
  up-to-date `objects`/`net_cache` table is sufficient to *walk* the stream even
  past attributes a given consumer ignores. A decoder that can't resolve a
  property id for the running build will desync — hence the pinned parser.
- **Non-standard maps/modes** (Hoops, Dropshot, Rumble, …) share the container and
  stream format but change arena geometry and add mode actors; geometry-derived
  analysis is gated by `field::classify_map` (`MapClass::NonStandard`).

---

## 9. Notable omissions — what the format does *not* carry

Commonly assumed to be in a replay, but **not reliably present**:

- **Competitive rank / division / MMR.** *Not in the file.* The only rank-adjacent
  attribute in the entire schema is `TAGame.PRI_TA:SkillTier` (a `FlaggedByte`),
  and in practice it is **declared but never populated** — both committed *ranked*
  samples list it in the class table yet emit **zero** `SkillTier` updates, and it
  is `0`/absent in the general case. There is **no** tier/division/MMR field on the
  `PlayerStats[]` header row either. `TAGame.GameEvent_TA:BotSkill` is **AI bot
  difficulty**, not player rank. Consequently rank must be sourced **out of band**
  (the Rocket League / tracker APIs, or the uploader's account at upload time) —
  which is exactly what ballchasing does, and what this repo's corpus does
  (per-player `ranks` live in `assets/corpus/manifest.json`, fetched from
  ballchasing's API, **not** parsed from the replay). **[stable]**
- **Raw button inputs** beyond what's replicated. Throttle, steer, and handbrake
  *are* replicated (§4.3); discrete **jump / boost / air-roll button presses** are
  not first-class — they're inferred from the component activation attributes
  (`CarComponent_*:ReplicatedActive`) and kinematics, not read as an input log.
- **Account names of anonymized players** (`bAnonymizeToOpponents/Teammates` →
  `AnonymizedName`), and **chat / voice** content (only `CurrentVoiceRoom`
  presence, not audio). **[stable]**
- **Authoritative possession / touch / pass labels.** The game ships a few
  counters (`PossessionClears/Denials/Steals`, `KeepUpPossessions`) and the
  `StatEvent` feed, but per-touch *type* (pass, dribble, 50/50) is a derivation
  (catalog Layer C), not a stored fact.

---

## 10. References

- **boxcars** — Rust parser used here: <https://github.com/nickbabcock/boxcars>
- **rattletrap** — Haskell parser/generator (ballchasing lineage):
  <https://github.com/tfausak/rattletrap>
- **jjbott/RocketLeagueReplayParser** — the original C# reverse-engineering, with
  the most prose on the bit layout.
- This workspace: [`src/decode/`](../../crates/replay-analyzer/src/decode) (the port + boxcars adapter),
  [`src/analyze/reconstruct.rs`](../../crates/replay-analyzer/src/analyze/reconstruct.rs) (the obligations of
  §5), [`docs/ballchasing-analyzer-teardown.md`] *(private repo)*
  (the stats layer on top).

---

## Appendix A — Complete attribute reference

The full **data dictionary** of the network stream: every `object string →
attribute type` mapping the format can carry, as recognized by **boxcars**
(`src/data.rs::ATTRIBUTES`, **298** entries). This is the *universe* — a given
replay only exercises the attributes that occurred in that match, but any of
these may appear. The set grows as Rocket League ships features (new modes,
cosmetics, stats); **boxcars/rattletrap are the source of truth** for the current
list, and an unknown property id against an outdated table desyncs the decode
(§8). Grouped by owning actor; **[stable]** as a structure, **[versioned]** in
membership.

Per-player relevance: groups **A.2–A.6** (and A.10 in non-Soccar modes) are the
per-player surface — see [`player-metrics-catalog.md`](player-metrics-catalog.md)
for what each enables. A.1/A.7–A.9 are match/team/ball context.

### Attribute-type legend

What each `Attribute` payload carries (the right column of the tables below):

| Type | Payload |
|---|---|
| `Boolean` / `Byte` / `Int` / `Int64` / `Float` / `QWord` | scalar of that kind (`Byte` = `u8`) |
| `String` / `QWordString` | text / stringized 64-bit id |
| `Enum` / `FlaggedByte` / `GameMode` / `DamageState` | small enumerated state |
| `ActiveActor` | a link to another actor (id + active flag) — identity/ownership edges |
| `UniqueId` | platform + platform-id + local-id (account identity) |
| `RigidBody` | position + quaternion + linear/angular velocity + sleeping |
| `Location` / `RotationTag` / `Impulse` | a compressed 3-vector / rotation / impulse |
| `ReplicatedBoost` | boost gauge (grant + amount byte) |
| `Pickup` / `PickupNew` / `PickupInfo` | pad/item pickup: instigator + picked-up flag |
| `Demolish` / `DemolishExtended` / `DemolishFx` | attacker + victim (+ fx) |
| `CamSettings` | FOV, distance, height, angle, stiffness, swivel/transition speed |
| `Loadout` / `LoadoutOnline` / `TeamLoadout` / `LoadoutsOnline` / `TeamPaint` / `ClubColors` | cosmetics: bodies, paints, products, colors |
| `Title` / `RepStatTitle` | scoreboard titles / stat-title medals |
| `PartyLeader` / `Reservation` / `PlayerHistoryKey` | party, seat reservation, player-history ref |
| `StatEvent` | a named, player-attributed in-game stat event (Goal, Save, Demolish, …) |
| `Explosion` / `ExtendedExplosion` / `AppliedDamage` / `MusicStinger` / `Welded` / `LogoData` / `PrivateMatchSettings` | goal-explosion / Dropshot damage / weld / club logo / private-match rules |

### The complete table

### A.1 Engine / system / game-replication-info  (26)

| Object string | Attribute type |
|---|---|
| `Engine.Actor:DrawScale` | `Float` |
| `Engine.Actor:RemoteRole` | `Enum` |
| `Engine.Actor:Role` | `Enum` |
| `Engine.Actor:Rotation` | `RotationTag` |
| `Engine.Actor:bBlockActors` | `Boolean` |
| `Engine.Actor:bCollideActors` | `Boolean` |
| `Engine.Actor:bCollideWorld` | `Boolean` |
| `Engine.Actor:bHidden` | `Boolean` |
| `Engine.Actor:bNetOwner` | `Boolean` |
| `Engine.Actor:bTearOff` | `Boolean` |
| `Engine.GameReplicationInfo:GameClass` | `ActiveActor` |
| `Engine.GameReplicationInfo:ServerName` | `String` |
| `Engine.GameReplicationInfo:bMatchIsOver` | `Boolean` |
| `Engine.ReplicatedActor_ORS:ReplicatedOwner` | `ActiveActor` |
| `ProjectX.GRI_X:GameServerID` | `QWordString` |
| `ProjectX.GRI_X:MatchGUID` | `String` |
| `ProjectX.GRI_X:MatchGuid` | `String` |
| `ProjectX.GRI_X:ReplicatedGameMutatorIndex` | `Int` |
| `ProjectX.GRI_X:ReplicatedGamePlaylist` | `Int` |
| `ProjectX.GRI_X:ReplicatedServerRegion` | `String` |
| `ProjectX.GRI_X:Reservations` | `Reservation` |
| `ProjectX.GRI_X:bGameStarted` | `Boolean` |
| `TAGame.GRI_TA:NewDedicatedServerIP` | `String` |
| `TAGame.GRI_TA:bAllowTargetFind` | `Boolean` |
| `TAGame.MaxTimeWarningData_TA:EndGameEpochTime` | `Int64` |
| `TAGame.MaxTimeWarningData_TA:EndGameWarningEpochTime` | `Int64` |

### A.2 Player (PRI) — identity, scoreboard, camera/loadout refs  (72)

| Object string | Attribute type |
|---|---|
| `Engine.PlayerReplicationInfo:Ping` | `Byte` |
| `Engine.PlayerReplicationInfo:PlayerID` | `Int` |
| `Engine.PlayerReplicationInfo:PlayerName` | `String` |
| `Engine.PlayerReplicationInfo:RemoteUserData` | `String` |
| `Engine.PlayerReplicationInfo:Score` | `Int` |
| `Engine.PlayerReplicationInfo:Team` | `ActiveActor` |
| `Engine.PlayerReplicationInfo:UniqueId` | `UniqueId` |
| `Engine.PlayerReplicationInfo:bAdmin` | `Boolean` |
| `Engine.PlayerReplicationInfo:bBot` | `Boolean` |
| `Engine.PlayerReplicationInfo:bIsSpectator` | `Boolean` |
| `Engine.PlayerReplicationInfo:bReadyToPlay` | `Boolean` |
| `Engine.PlayerReplicationInfo:bTimedOut` | `Boolean` |
| `Engine.PlayerReplicationInfo:bWaitingPlayer` | `Boolean` |
| `TAGame.PRI_TA:AnonymizedName` | `String` |
| `TAGame.PRI_TA:BotBannerProductID` | `Int` |
| `TAGame.PRI_TA:BotProductName` | `Int` |
| `TAGame.PRI_TA:CameraPitch` | `Byte` |
| `TAGame.PRI_TA:CameraSettings` | `CamSettings` |
| `TAGame.PRI_TA:CameraYaw` | `Byte` |
| `TAGame.PRI_TA:CarDemolitions` | `Int` |
| `TAGame.PRI_TA:ClientLoadout` | `Loadout` |
| `TAGame.PRI_TA:ClientLoadoutOnline` | `LoadoutOnline` |
| `TAGame.PRI_TA:ClientLoadouts` | `TeamLoadout` |
| `TAGame.PRI_TA:ClientLoadoutsOnline` | `LoadoutsOnline` |
| `TAGame.PRI_TA:ClubID` | `Int64` |
| `TAGame.PRI_TA:CurrentVoiceRoom` | `String` |
| `TAGame.PRI_TA:EpicPUID` | `String` |
| `TAGame.PRI_TA:KeepUpPossessions` | `Int` |
| `TAGame.PRI_TA:MatchAssists` | `Int` |
| `TAGame.PRI_TA:MatchBreakoutDamage` | `Int` |
| `TAGame.PRI_TA:MatchDemolishes` | `Int` |
| `TAGame.PRI_TA:MatchGoals` | `Int` |
| `TAGame.PRI_TA:MatchSaves` | `Int` |
| `TAGame.PRI_TA:MatchScore` | `Int` |
| `TAGame.PRI_TA:MatchShots` | `Int` |
| `TAGame.PRI_TA:MaxTimeTillItem` | `Int` |
| `TAGame.PRI_TA:PartyLeader` | `PartyLeader` |
| `TAGame.PRI_TA:PawnType` | `Byte` |
| `TAGame.PRI_TA:PersistentCamera` | `ActiveActor` |
| `TAGame.PRI_TA:PlayerHistoryKey` | `PlayerHistoryKey` |
| `TAGame.PRI_TA:PlayerHistoryValid` | `Boolean` |
| `TAGame.PRI_TA:PossessionClears` | `Int` |
| `TAGame.PRI_TA:PossessionDenials` | `Int` |
| `TAGame.PRI_TA:PossessionSteals` | `Int` |
| `TAGame.PRI_TA:PrimaryTitle` | `Title` |
| `TAGame.PRI_TA:RepStatTitles` | `RepStatTitle` |
| `TAGame.PRI_TA:ReplicatedGameEvent` | `ActiveActor` |
| `TAGame.PRI_TA:ReplicatedWorstNetQualityBeyondLatency` | `Byte` |
| `TAGame.PRI_TA:SecondaryTitle` | `Title` |
| `TAGame.PRI_TA:SelfDemolitions` | `Int` |
| `TAGame.PRI_TA:SkillTier` | `FlaggedByte` |
| `TAGame.PRI_TA:SpectatorShortcut` | `Int` |
| `TAGame.PRI_TA:SteeringSensitivity` | `Float` |
| `TAGame.PRI_TA:TimeTillItem` | `Int` |
| `TAGame.PRI_TA:Title` | `Int` |
| `TAGame.PRI_TA:TotalGameTimePlayed` | `Float` |
| `TAGame.PRI_TA:TotalIdleTime` | `Float` |
| `TAGame.PRI_TA:TotalXP` | `Int` |
| `TAGame.PRI_TA:ViralItemActor` | `ActiveActor` |
| `TAGame.PRI_TA:bAnonymizeToOpponents` | `Boolean` |
| `TAGame.PRI_TA:bAnonymizeToTeammates` | `Boolean` |
| `TAGame.PRI_TA:bIdleBanned` | `Boolean` |
| `TAGame.PRI_TA:bIsDistracted` | `Boolean` |
| `TAGame.PRI_TA:bIsInSplitScreen` | `Boolean` |
| `TAGame.PRI_TA:bMatchMVP` | `Boolean` |
| `TAGame.PRI_TA:bOnlineLoadoutSet` | `Boolean` |
| `TAGame.PRI_TA:bOnlineLoadoutsSet` | `Boolean` |
| `TAGame.PRI_TA:bReady` | `Boolean` |
| `TAGame.PRI_TA:bReceivedAnonymizationSettings` | `Boolean` |
| `TAGame.PRI_TA:bUsingBehindView` | `Boolean` |
| `TAGame.PRI_TA:bUsingItems` | `Boolean` |
| `TAGame.PRI_TA:bUsingSecondaryCamera` | `Boolean` |

### A.3 Pawn / rigid body (per-frame physical state)  (12)

| Object string | Attribute type |
|---|---|
| `Engine.Pawn:HealthMax` | `Int` |
| `Engine.Pawn:PlayerReplicationInfo` | `ActiveActor` |
| `Engine.Pawn:RemoteViewPitch` | `Byte` |
| `Engine.Pawn:bFastAttachedMove` | `Boolean` |
| `Engine.Pawn:bIsCrouched` | `Boolean` |
| `Engine.Pawn:bUsedByMatinee` | `Boolean` |
| `TAGame.RBActor_TA:ReplicatedRBState` | `RigidBody` |
| `TAGame.RBActor_TA:TeleportCounter` | `Byte` |
| `TAGame.RBActor_TA:WeldedInfo` | `Welded` |
| `TAGame.RBActor_TA:bFrozen` | `Boolean` |
| `TAGame.RBActor_TA:bIgnoreSyncing` | `Boolean` |
| `TAGame.RBActor_TA:bReplayActor` | `Boolean` |

### A.4 Car / vehicle (inputs, demolitions, scale)  (22)

| Object string | Attribute type |
|---|---|
| `TAGame.Car_TA:AddedBallForceMultiplier` | `Float` |
| `TAGame.Car_TA:AddedCarForceMultiplier` | `Float` |
| `TAGame.Car_TA:AttachedPickup` | `ActiveActor` |
| `TAGame.Car_TA:ClubColors` | `ClubColors` |
| `TAGame.Car_TA:DodgesRefreshedCounter` | `Int` |
| `TAGame.Car_TA:ReplicatedCarMaxLinearSpeedScale` | `Float` |
| `TAGame.Car_TA:ReplicatedCarScale` | `Float` |
| `TAGame.Car_TA:ReplicatedDemolish` | `Demolish` |
| `TAGame.Car_TA:ReplicatedDemolishExtended` | `DemolishExtended` |
| `TAGame.Car_TA:ReplicatedDemolishGoalExplosion` | `DemolishFx` |
| `TAGame.Car_TA:ReplicatedDemolish_CustomFX` | `DemolishFx` |
| `TAGame.Car_TA:RumblePickups` | `ActiveActor` |
| `TAGame.Car_TA:TeamPaint` | `TeamPaint` |
| `TAGame.Car_TA:bUnlimitedJumps` | `Boolean` |
| `TAGame.Car_TA:bUnlimitedTimeForDodge` | `Boolean` |
| `TAGame.Vehicle_TA:InputRestriction` | `Byte` |
| `TAGame.Vehicle_TA:ReplicatedSteer` | `Byte` |
| `TAGame.Vehicle_TA:ReplicatedThrottle` | `Byte` |
| `TAGame.Vehicle_TA:bDriving` | `Boolean` |
| `TAGame.Vehicle_TA:bHasPostMatchCelebration` | `Boolean` |
| `TAGame.Vehicle_TA:bPodiumMode` | `Boolean` |
| `TAGame.Vehicle_TA:bReplicatedHandbrake` | `Boolean` |

### A.5 Car components (boost / jump / dodge / flip / torque)  (22)

| Object string | Attribute type |
|---|---|
| `TAGame.CarComponent_AirActivate_TA:AirActivateCount` | `Int` |
| `TAGame.CarComponent_Boost_TA:BoostModifier` | `Float` |
| `TAGame.CarComponent_Boost_TA:BoostRestriction` | `Byte` |
| `TAGame.CarComponent_Boost_TA:RechargeDelay` | `Float` |
| `TAGame.CarComponent_Boost_TA:RechargeRate` | `Float` |
| `TAGame.CarComponent_Boost_TA:ReplicatedBoost` | `ReplicatedBoost` |
| `TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount` | `Byte` |
| `TAGame.CarComponent_Boost_TA:UnlimitedBoostRefCount` | `Int` |
| `TAGame.CarComponent_Boost_TA:bNoBoost` | `Boolean` |
| `TAGame.CarComponent_Boost_TA:bRechargeGroundOnly` | `Boolean` |
| `TAGame.CarComponent_Boost_TA:bUnlimitedBoost` | `Boolean` |
| `TAGame.CarComponent_Dodge_KO_TA:DodgeRotationCompressed` | `Int` |
| `TAGame.CarComponent_Dodge_TA:DodgeImpulse` | `Location` |
| `TAGame.CarComponent_Dodge_TA:DodgeTorque` | `Location` |
| `TAGame.CarComponent_DoubleJump_TA:DoubleJumpImpulse` | `Location` |
| `TAGame.CarComponent_FlipCar_TA:FlipCarTime` | `Float` |
| `TAGame.CarComponent_FlipCar_TA:bFlipRight` | `Boolean` |
| `TAGame.CarComponent_TA:ReplicatedActive` | `Byte` |
| `TAGame.CarComponent_TA:ReplicatedActivityTime` | `Float` |
| `TAGame.CarComponent_TA:Vehicle` | `ActiveActor` |
| `TAGame.CarComponent_Torque_TA:ReplicatedTorqueInput` | `Int` |
| `TAGame.CarComponent_Torque_TA:TorqueScale` | `Float` |

### A.6 Camera / view  (8)

| Object string | Attribute type |
|---|---|
| `TAGame.CameraSettingsActor_TA:CameraPitch` | `Byte` |
| `TAGame.CameraSettingsActor_TA:CameraYaw` | `Byte` |
| `TAGame.CameraSettingsActor_TA:PRI` | `ActiveActor` |
| `TAGame.CameraSettingsActor_TA:ProfileSettings` | `CamSettings` |
| `TAGame.CameraSettingsActor_TA:bMouseCameraToggleEnabled` | `Boolean` |
| `TAGame.CameraSettingsActor_TA:bUsingBehindView` | `Boolean` |
| `TAGame.CameraSettingsActor_TA:bUsingSecondaryCamera` | `Boolean` |
| `TAGame.CameraSettingsActor_TA:bUsingSwivel` | `Boolean` |

### A.7 Team  (8)

| Object string | Attribute type |
|---|---|
| `Engine.TeamInfo:Score` | `Int` |
| `TAGame.Team_Soccar_TA:GameScore` | `Int` |
| `TAGame.Team_TA:ClubColors` | `ClubColors` |
| `TAGame.Team_TA:ClubID` | `Int64` |
| `TAGame.Team_TA:CustomTeamName` | `String` |
| `TAGame.Team_TA:Difficulty` | `Int` |
| `TAGame.Team_TA:GameEvent` | `ActiveActor` |
| `TAGame.Team_TA:LogoData` | `LogoData` |

### A.8 Ball  (33)

| Object string | Attribute type |
|---|---|
| `TAGame.BallKeepUpComponent_TA:BallOwner` | `ActiveActor` |
| `TAGame.BallKeepUpComponent_TA:KeepUpState` | `Byte` |
| `TAGame.BallKeepUpComponent_TA:Score` | `Int` |
| `TAGame.Ball_Breakout_TA:AppliedDamage` | `AppliedDamage` |
| `TAGame.Ball_Breakout_TA:DamageIndex` | `Int` |
| `TAGame.Ball_Breakout_TA:LastTeamTouch` | `Byte` |
| `TAGame.Ball_Fire_TA:TeamNumChangeTimestamp` | `Float` |
| `TAGame.Ball_God_TA:TargetSpeed` | `Float` |
| `TAGame.Ball_Haunted_TA:DeactivatedGoalIndex` | `Byte` |
| `TAGame.Ball_Haunted_TA:LastTeamTouch` | `Byte` |
| `TAGame.Ball_Haunted_TA:ReplicatedBeamBrokenValue` | `Byte` |
| `TAGame.Ball_Haunted_TA:TotalActiveBeams` | `Byte` |
| `TAGame.Ball_Haunted_TA:bIsBallBeamed` | `Boolean` |
| `TAGame.Ball_Spawner_TA:SpawnDelaySeconds` | `Float` |
| `TAGame.Ball_Spawner_TA:SpawnedBall` | `ActiveActor` |
| `TAGame.Ball_TA:AdditionalCarGroundBounceScaleXY` | `Float` |
| `TAGame.Ball_TA:AdditionalCarGroundBounceScaleZ` | `Float` |
| `TAGame.Ball_TA:AirResistance` | `Location` |
| `TAGame.Ball_TA:BallHitSpinScale` | `Float` |
| `TAGame.Ball_TA:GameBallIndex` | `Int` |
| `TAGame.Ball_TA:GameEvent` | `ActiveActor` |
| `TAGame.Ball_TA:HitTeamNum` | `Byte` |
| `TAGame.Ball_TA:MagnusMinSpeed` | `Float` |
| `TAGame.Ball_TA:ReplicatedAddedCarBounceScale` | `Float` |
| `TAGame.Ball_TA:ReplicatedBallGravityScale` | `Float` |
| `TAGame.Ball_TA:ReplicatedBallMaxLinearSpeedScale` | `Float` |
| `TAGame.Ball_TA:ReplicatedBallScale` | `Float` |
| `TAGame.Ball_TA:ReplicatedExplosionData` | `Explosion` |
| `TAGame.Ball_TA:ReplicatedExplosionDataExtended` | `ExtendedExplosion` |
| `TAGame.Ball_TA:ReplicatedPhysMatOverride` | `ActiveActor` |
| `TAGame.Ball_TA:ReplicatedWorldBounceScale` | `Float` |
| `TAGame.Ball_TA:bPossessionEnabled` | `Boolean` |
| `TAGame.Ball_TA:bWarnBallReset` | `Boolean` |

### A.9 Game event / rules / match state  (55)

| Object string | Attribute type |
|---|---|
| `TAGame.CrowdActor_TA:GameEvent` | `ActiveActor` |
| `TAGame.CrowdActor_TA:ModifiedNoise` | `Float` |
| `TAGame.CrowdActor_TA:ReplicatedCountDownNumber` | `Int` |
| `TAGame.CrowdActor_TA:ReplicatedOneShotSound` | `ActiveActor` |
| `TAGame.CrowdActor_TA:ReplicatedRoundCountDownNumber` | `Int` |
| `TAGame.CrowdManager_TA:GameEvent` | `ActiveActor` |
| `TAGame.CrowdManager_TA:ReplicatedGlobalOneShotSound` | `ActiveActor` |
| `TAGame.GameEvent_SoccarPrivate_TA:MatchSettings` | `PrivateMatchSettings` |
| `TAGame.GameEvent_Soccar_TA:GameTime` | `Int` |
| `TAGame.GameEvent_Soccar_TA:GameWinner` | `ActiveActor` |
| `TAGame.GameEvent_Soccar_TA:MVP` | `ActiveActor` |
| `TAGame.GameEvent_Soccar_TA:MatchWinner` | `ActiveActor` |
| `TAGame.GameEvent_Soccar_TA:MaxScore` | `Int` |
| `TAGame.GameEvent_Soccar_TA:ReplicatedMusicStinger` | `MusicStinger` |
| `TAGame.GameEvent_Soccar_TA:ReplicatedScoredOnTeam` | `Byte` |
| `TAGame.GameEvent_Soccar_TA:ReplicatedServerPerformanceState` | `Byte` |
| `TAGame.GameEvent_Soccar_TA:ReplicatedStatEvent` | `StatEvent` |
| `TAGame.GameEvent_Soccar_TA:RoundNum` | `Int` |
| `TAGame.GameEvent_Soccar_TA:SecondsRemaining` | `Int` |
| `TAGame.GameEvent_Soccar_TA:SeriesLength` | `Int` |
| `TAGame.GameEvent_Soccar_TA:SubRulesArchetype` | `ActiveActor` |
| `TAGame.GameEvent_Soccar_TA:TotalGameBalls` | `Int` |
| `TAGame.GameEvent_Soccar_TA:bAllowHonorDuels` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bBallHasBeenHit` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bClubMatch` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bDisableCrowdSound` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bFullClubMatch` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bFullMatchWinnerDecided` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bGoalsEnabled` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bMatchCreatorAdminEnabled` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bMatchEnded` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bNoContest` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bOverTime` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bReadyToStartGame` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bShouldSpawnGoalIndicators` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bShowIntroScene` | `Boolean` |
| `TAGame.GameEvent_Soccar_TA:bUnlimitedTime` | `Boolean` |
| `TAGame.GameEvent_TA:BotSkill` | `Int` |
| `TAGame.GameEvent_TA:GameMode` | `GameMode` |
| `TAGame.GameEvent_TA:MatchStartEpoch` | `Int64` |
| `TAGame.GameEvent_TA:MatchTypeClass` | `ActiveActor` |
| `TAGame.GameEvent_TA:ReplicatedGameStateTimeRemaining` | `Int` |
| `TAGame.GameEvent_TA:ReplicatedRoundCountDownNumber` | `Int` |
| `TAGame.GameEvent_TA:ReplicatedStateIndex` | `Byte` |
| `TAGame.GameEvent_TA:ReplicatedStateName` | `Int` |
| `TAGame.GameEvent_TA:RichPresenceString` | `String` |
| `TAGame.GameEvent_TA:bAllowReadyUp` | `Boolean` |
| `TAGame.GameEvent_TA:bAlwaysShowMatchTypeLabel` | `Boolean` |
| `TAGame.GameEvent_TA:bCanVoteToForfeit` | `Boolean` |
| `TAGame.GameEvent_TA:bHasLeaveMatchPenalty` | `Boolean` |
| `TAGame.GameEvent_TA:bIsBotMatch` | `Boolean` |
| `TAGame.GameEvent_Team_TA:MaxTeamSize` | `Int` |
| `TAGame.GameEvent_Team_TA:bDisableMutingOtherTeam` | `Boolean` |
| `TAGame.GameEvent_Team_TA:bDisableQuickChat` | `Boolean` |
| `TAGame.GameEvent_Team_TA:bForfeit` | `Boolean` |

### A.10 Pickups / mode actors / per-mode player stats  (40)

| Object string | Attribute type |
|---|---|
| `TAGame.BreakOutActor_Platform_TA:DamageState` | `DamageState` |
| `TAGame.Cannon_TA:FireCount` | `Byte` |
| `TAGame.Cannon_TA:Pitch` | `Float` |
| `TAGame.Car_KnockOut_TA:ReplicatedImpulse` | `Impulse` |
| `TAGame.Car_KnockOut_TA:ReplicatedStateChanged` | `Byte` |
| `TAGame.Car_KnockOut_TA:ReplicatedStateName` | `Int` |
| `TAGame.Car_KnockOut_TA:UsedAttackComponent` | `ActiveActor` |
| `TAGame.KeepUpIndicator_TA:ComponentOwner` | `ActiveActor` |
| `TAGame.PRI_KnockOut_TA:Blocks` | `Int` |
| `TAGame.PRI_KnockOut_TA:DamageCaused` | `Int` |
| `TAGame.PRI_KnockOut_TA:EliminationOrder` | `Int` |
| `TAGame.PRI_KnockOut_TA:Grabs` | `Int` |
| `TAGame.PRI_KnockOut_TA:Hits` | `Int` |
| `TAGame.PRI_KnockOut_TA:KnockoutDeaths` | `Int` |
| `TAGame.PRI_KnockOut_TA:Knockouts` | `Int` |
| `TAGame.PRI_KnockOut_TA:bIsActiveMVP` | `Boolean` |
| `TAGame.PRI_KnockOut_TA:bIsEliminated` | `Boolean` |
| `TAGame.PickupTimer_TA:MaxTimeTillItem` | `Int` |
| `TAGame.PickupTimer_TA:TimeTillItem` | `Int` |
| `TAGame.PlayerStart_Platform_TA:bActive` | `Boolean` |
| `TAGame.RumblePickups_TA:AttachedPickup` | `ActiveActor` |
| `TAGame.RumblePickups_TA:ConcurrentItemCount` | `Int` |
| `TAGame.RumblePickups_TA:PickupInfo` | `PickupInfo` |
| `TAGame.SpecialPickup_BallFreeze_TA:RepOrigSpeed` | `Float` |
| `TAGame.SpecialPickup_BallVelcro_TA:AttachTime` | `Float` |
| `TAGame.SpecialPickup_BallVelcro_TA:BreakTime` | `Float` |
| `TAGame.SpecialPickup_BallVelcro_TA:bBroken` | `Boolean` |
| `TAGame.SpecialPickup_BallVelcro_TA:bHit` | `Boolean` |
| `TAGame.SpecialPickup_Football_TA:WeldedBall` | `ActiveActor` |
| `TAGame.SpecialPickup_Rugby_TA:bBallWelded` | `Boolean` |
| `TAGame.SpecialPickup_Targeted_TA:Targeted` | `ActiveActor` |
| `TAGame.Stunlock_TA:Car` | `ActiveActor` |
| `TAGame.Stunlock_TA:MashTime` | `Float` |
| `TAGame.Stunlock_TA:MaxStunTime` | `Float` |
| `TAGame.Stunlock_TA:StunTimeRemaining` | `Float` |
| `TAGame.VehiclePickup_TA:NewReplicatedPickupData` | `PickupNew` |
| `TAGame.VehiclePickup_TA:ReplicatedPickupData` | `Pickup` |
| `TAGame.VehiclePickup_TA:bNoPickup` | `Boolean` |
| `TAGame.ViralItemActor_TA:ClientFXInfectedType` | `Byte` |
| `TAGame.ViralItemActor_TA:InfectedStatus` | `Byte` |

---

## Appendix B — Header property reference

The header is an open-ended property map (§3), but online matches carry a stable
standard set. Below is the union observed across the committed samples (26 keys);
match type and version can add or drop a few (private/tournament/LAN, freeplay).
Types are the `HeaderProp` kind. **[stable]** as a set, **[versioned]** at the
margins.

| Key | Type | Meaning |
|---|---|---|
| `TeamSize` | Int | players per team |
| `UnfairTeamSize` | Int | uneven-teams handicap (0 if balanced) |
| `PrimaryPlayerTeam` | Int | the recording player's team |
| `Team0Score` / `Team1Score` | Int | final score (blue / orange) |
| `Goals` | Array | scored-goal rows (schema below) |
| `HighLights` | Array | highlight-reel markers (schema below) |
| `PlayerStats` | Array | per-player scoreboard rows (schema below) |
| `PlayerName` | Str | the recording/primary player's name |
| `ReplayName` | Str | user-set replay title (often empty) |
| `MapName` | Name | arena id, e.g. `Stadium_P` |
| `Date` | Str | local timestamp `YYYY-MM-DD HH-MM-SS` |
| `Id` | Str | replay GUID |
| `MatchType` | Name | `Online` / `Offline` / `Private` / `Season` / `LAN` … |
| `RecordFPS` | Float | record rate (~30) |
| `KeyframeDelay` | Float | seconds between keyframes |
| `MaxChannels` | Int | actor-channel cap (actor-id bit width, §4) |
| `NumFrames` | Int | network-stream frame count |
| `ReplayVersion` | Int | replay format version |
| `ReplayLastSaveVersion` | Int | version that last saved the file |
| `GameVersion` | Int | game version flag |
| `BuildID` / `Changelist` | Int | build identifiers |
| `BuildVersion` | Str | build version string |
| `MaxReplaySizeMB` / `ReserveMegabytes` | Int | size-budget hints |

### Array-row schemas

```
PlayerStats[] : { Name:Str, Platform:Byte(enum, e.g. OnlinePlatform_Steam/Epic/PS4/Dingo),
                  OnlineID:QWord, Team:Int(0/1), Score:Int, Goals:Int, Assists:Int,
                  Saves:Int, Shots:Int, bBot:Bool }
Goals[]       : { frame:Int (network-frame index), PlayerName:Str, PlayerTeam:Int }
HighLights[]  : { frame:Int, CarName:Name, BallName:Name [, GoalActorName:Name] }
```

`PlayerStats[]` is the **authoritative scoreboard + account identity**
(`OnlineID` + `Platform` are the platform identity; `bBot` flags AI). `Goals[]`
is the authoritative per-goal scorer/team/time. `HighLights[]` drives the
in-client highlight reel (the car/ball actors at each marked frame).

---

## Appendix C — Archetype / class reference

The other half of the `objects` table (§4.1): the **classes and archetypes** that
spawn as actors, complementing the *attribute* dictionary of Appendix A. From
boxcars' `PARENT_CLASSES` (archetype → class, **285** entries) and `SPAWN_STATS`
(which classes carry an initial trajectory). **[stable]** as a taxonomy,
**[versioned]** in membership — RL adds archetypes every season (new balls, cars,
items, modes), so this is the set boxcars 0.11.3 knows; the source of truth tracks
the live game.

**Archetype vs. instance.** A *spawn* references an **archetype** (e.g.
`Archetypes.Ball.Ball_Default`), which resolves through `PARENT_CLASSES` to a
**class** (`TAGame.Ball_TA`); the class determines which attributes (Appendix A)
it can replicate and its spawn trajectory (below). A real replay *also* contains
per-map **instance** names in `objects` — e.g.
`Stadium_P.TheWorld:PersistentLevel.VehiclePickup_Boost_TA_42` (one of the 34
pads) or `…:CarComponent_Boost_TA_0` — which are level-placed instances of these
classes, not archetypes, and so vary by map. Those are not catalogued here (they
are generated per level), but each is an instance of a class below.

### Spawn trajectory (`SPAWN_STATS`)

At spawn an actor reads an **initial trajectory** per its class (inherited up the
`PARENT_CLASSES` chain to the nearest entry here):

| Class | Initial trajectory at spawn |
|---|---|
| `TAGame.RBActor_TA` (balls + cars derive from it) | **Location + Rotation** |
| `TAGame.KeepUpIndicator_TA` | Location + Rotation |
| `Engine.Actor` | Location only |
| `Engine.ZoneInfo`, `TAGame.VehiclePickup_Boost_TA`, `TAGame.CrowdActor_TA`, `TAGame.CrowdManager_TA`, `TAGame.BreakOutActor_Platform_TA`, `TAGame.PlayerStart_Platform_TA`, `TAGame.InMapScoreboard_TA`, `TAGame.HauntedBallTrapTrigger_TA` | None (no position at spawn) |

So the ball and every car (both `RBActor_TA` descendants) spawn with a position
and rotation; boost pads, crowd, scoreboard and trigger actors spawn with neither
(they're static or attribute-only). This is the rule §4.2's "initial trajectory"
points at.

### Archetype → class catalog (`PARENT_CLASSES`)

### C.1 Ball (+ mode balls)  (56)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.Ball.BallComponent_KeepUp` | `TAGame.BallKeepUpComponent_TA` |
| `Archetypes.Ball.Ball_Anniversary` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_BasketBall` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_BasketBall_Mutator` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Basketball` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Beachball` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Breakout` | `TAGame.Ball_Breakout_TA` |
| `Archetypes.Ball.Ball_Default` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Ekin` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Fire` | `TAGame.Ball_Fire_TA` |
| `Archetypes.Ball.Ball_Fire_Obstacle` | `TAGame.Ball_Fire_TA` |
| `Archetypes.Ball.Ball_Football` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_God` | `TAGame.Ball_God_TA` |
| `Archetypes.Ball.Ball_Haunted` | `TAGame.Ball_Haunted_TA` |
| `Archetypes.Ball.Ball_PizzaPuck` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Puck` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_RingSpawner` | `TAGame.Ball_Spawner_TA` |
| `Archetypes.Ball.Ball_Score` | `TAGame.Ball_Breakout_TA` |
| `Archetypes.Ball.Ball_Shoe` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_SpookyBalloon` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Strike` | `TAGame.Ball_TA` |
| `Archetypes.Ball.Ball_Training` | `TAGame.Ball_Tutorial_TA` |
| `Archetypes.Ball.Ball_Trajectory` | `TAGame.Ball_Trajectory_TA` |
| `Archetypes.Ball.Ball_Tutorial` | `TAGame.Ball_Tutorial_TA` |
| `Archetypes.Ball.Ball_WorldCup` | `TAGame.Ball_TA` |
| `Archetypes.Ball.CubeBall` | `TAGame.Ball_TA` |
| `Archetypes.Ball.ball_luminousairplane` | `TAGame.Ball_TA` |
| `Archetypes.GameEvent.GameEvent_Basketball` | `TAGame.GameEvent_Soccar_TA` |
| `Archetypes.GameEvent.GameEvent_BasketballPrivate` | `TAGame.GameEvent_SoccarPrivate_TA` |
| `Archetypes.GameEvent.GameEvent_BasketballSplitscreen` | `TAGame.GameEvent_SoccarSplitscreen_TA` |
| `GameInfo_Basketball.GameInfo.GameInfo_Basketball:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_Basketball.GameInfo.GameInfo_Basketball:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_FootBall.GameInfo.GameInfo_FootBall:Archetype` | `TAGame.GameEvent_Football_TA` |
| `GameInfo_FootBall.GameInfo.GameInfo_FootBall:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_GodBall.GameInfo.GameInfo_GodBall:Archetype` | `TAGame.GameEvent_GodBall_TA` |
| `GameInfo_GodBall.GameInfo.GameInfo_GodBall:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_LTM_BeachBall.GameInfo.GameInfo_LTM_BeachBall:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_BeachBall.GameInfo.GameInfo_LTM_BeachBall:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_MagnusFutball.GameInfo.GameInfo_MagnusFutball:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_MagnusFutball.GameInfo.GameInfo_MagnusFutball:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `Haunted_TrainStation_P.TheWorld:PersistentLevel.HauntedBallTrapTrigger_TA_0` | `TAGame.HauntedBallTrapTrigger_TA` |
| `Haunted_TrainStation_P.TheWorld:PersistentLevel.HauntedBallTrapTrigger_TA_1` | `TAGame.HauntedBallTrapTrigger_TA` |
| `TAGame.BallKeepUpComponent_TA` | `Engine.ReplicatedActor_ORS` |
| `TAGame.Ball_Breakout_TA` | `TAGame.Ball_TA` |
| `TAGame.Ball_Fire_TA` | `TAGame.Ball_God_TA` |
| `TAGame.Ball_God_TA` | `TAGame.Ball_TA` |
| `TAGame.Ball_Haunted_TA` | `TAGame.Ball_TA` |
| `TAGame.Ball_Spawner_TA` | `Engine.Actor` |
| `TAGame.Ball_TA` | `TAGame.RBActor_TA` |
| `TAGame.Ball_Trajectory_TA` | `TAGame.Ball_TA` |
| `TAGame.Ball_Tutorial_TA` | `TAGame.Ball_TA` |
| `TAGame.GameEvent_Football_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_GodBall_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.HauntedBallTrapTrigger_TA` | `TAGame.DynamicMeshActor_TA` |
| `gameinfo_godball.GameInfo.gameinfo_godball:Archetype` | `TAGame.GameEvent_GodBall_TA` |
| `gameinfo_godball.GameInfo.gameinfo_godball:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |

### C.2 Car / vehicle  (8)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.Car.Car_Default` | `TAGame.Car_TA` |
| `Archetypes.Car.Car_PostGameLobby` | `TAGame.Car_Freeplay_TA` |
| `Mutators.Mutators.Mutators.FreePlay:CarArchetype` | `TAGame.Car_Freeplay_TA` |
| `Mutators.Mutators.Mutators.OnlineFreeplay:CarArchetype` | `TAGame.Car_Freeplay_TA` |
| `TAGame.Car_Freeplay_TA` | `TAGame.Car_TA` |
| `TAGame.Car_KnockOut_TA` | `TAGame.Car_TA` |
| `TAGame.Car_Season_TA` | `TAGame.Car_TA` |
| `TAGame.Default__Car_TA` | `TAGame.Car_TA` |

### C.3 Car components  (31)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.CarComponents.CarComponent_Boost` | `TAGame.CarComponent_Boost_TA` |
| `Archetypes.CarComponents.CarComponent_Dodge` | `TAGame.CarComponent_Dodge_TA` |
| `Archetypes.CarComponents.CarComponent_DoubleJump` | `TAGame.CarComponent_DoubleJump_TA` |
| `Archetypes.CarComponents.CarComponent_FlipCar` | `TAGame.CarComponent_FlipCar_TA` |
| `Archetypes.CarComponents.CarComponent_Jump` | `TAGame.CarComponent_Jump_TA` |
| `Archetypes.CarComponents.CarComponent_TerritoryDemolish` | `TAGame.CarComponent_TerritoryDemolish_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.Boost` | `TAGame.CarComponent_Boost_KO_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.Dodge` | `TAGame.CarComponent_Dodge_KO_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.DoubleJump` | `TAGame.CarComponent_DoubleJump_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.Flip` | `TAGame.CarComponent_FlipCar_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.Jump` | `TAGame.CarComponent_Jump_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.Torque` | `TAGame.CarComponent_Torque_TA` |
| `Archetypes.Mutators.Mutator_Robin:AutoFlip` | `TAGame.CarComponent_FlipCar_TA` |
| `Archetypes.Mutators.Mutator_Robin:DoubleJump` | `TAGame.CarComponent_DoubleJump_Robin_TA` |
| `Archetypes.Mutators.Mutator_Robin:Jump` | `TAGame.CarComponent_Jump_Robin_TA` |
| `TAGame.CarComponent_AirActivate_TA` | `TAGame.CarComponent_TA` |
| `TAGame.CarComponent_Boost_KO_TA` | `TAGame.CarComponent_Boost_TA` |
| `TAGame.CarComponent_Boost_TA` | `TAGame.CarComponent_AirActivate_TA` |
| `TAGame.CarComponent_Dodge_KO_TA` | `TAGame.CarComponent_Dodge_TA` |
| `TAGame.CarComponent_Dodge_TA` | `TAGame.CarComponent_AirActivate_TA` |
| `TAGame.CarComponent_DoubleJump_KO_TA` | `TAGame.CarComponent_DoubleJump_TA` |
| `TAGame.CarComponent_DoubleJump_Robin_TA` | `TAGame.CarComponent_DoubleJump_TA` |
| `TAGame.CarComponent_DoubleJump_TA` | `TAGame.CarComponent_AirActivate_TA` |
| `TAGame.CarComponent_FlipCar_TA` | `TAGame.CarComponent_TA` |
| `TAGame.CarComponent_Jump_Robin_TA` | `TAGame.CarComponent_Jump_TA` |
| `TAGame.CarComponent_Jump_TA` | `TAGame.CarComponent_TA` |
| `TAGame.CarComponent_TA` | `Engine.ReplicationInfo` |
| `TAGame.CarComponent_TerritoryDemolish_TA` | `TAGame.CarComponent_TA` |
| `TAGame.CarComponent_Torque_TA` | `TAGame.CarComponent_TA` |
| `TAGame.PickupTimer_TA` | `TAGame.CarComponent_TA` |
| `TAGame.SpecialPickup_TA` | `TAGame.CarComponent_TA` |

### C.4 Boost pads / pickups / Rumble items  (41)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.SpecialPickups.BM.SpecialPickup_BallFreeze_BM` | `TAGame.SpecialPickup_BallFreeze_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BallFreeze` | `TAGame.SpecialPickup_BallFreeze_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BallGrapplingHook` | `TAGame.SpecialPickup_GrapplingHook_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BallLasso` | `TAGame.SpecialPickup_BallLasso_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BallSpring` | `TAGame.SpecialPickup_BallCarSpring_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BallVelcro` | `TAGame.SpecialPickup_BallVelcro_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_Batarang` | `TAGame.SpecialPickup_Batarang_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_BoostOverride` | `TAGame.SpecialPickup_BoostOverride_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_CarSpring` | `TAGame.SpecialPickup_BallCarSpring_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_Football` | `TAGame.SpecialPickup_Football_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_GravityWell` | `TAGame.SpecialPickup_BallGravity_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_HauntedBallBeam` | `TAGame.SpecialPickup_HauntedBallBeam_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_Rugby` | `TAGame.SpecialPickup_Rugby_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_RugbyLightDark` | `TAGame.SpecialPickup_Rugby_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_StrongHit` | `TAGame.SpecialPickup_HitForce_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_Swapper` | `TAGame.SpecialPickup_Swapper_TA` |
| `Archetypes.SpecialPickups.SpecialPickup_Tornado` | `TAGame.SpecialPickup_Tornado_TA` |
| `GameInfo_LTM_DropshotRumble.GameInfo.GameInfo_LTM_DropshotRumble:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_DropshotRumble.GameInfo.GameInfo_LTM_DropshotRumble:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `TAGame.Default__PickupTimer_TA` | `TAGame.PickupTimer_TA` |
| `TAGame.Default__RumblePickups_TA` | `TAGame.RumblePickups_TA` |
| `TAGame.RumblePickups_TA` | `Engine.Actor` |
| `TAGame.SpecialPickup_BallCarSpring_TA` | `TAGame.SpecialPickup_Spring_TA` |
| `TAGame.SpecialPickup_BallFreeze_TA` | `TAGame.SpecialPickup_Targeted_TA` |
| `TAGame.SpecialPickup_BallGravity_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_BallLasso_TA` | `TAGame.SpecialPickup_Spring_TA` |
| `TAGame.SpecialPickup_BallVelcro_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_Batarang_TA` | `TAGame.SpecialPickup_BallLasso_TA` |
| `TAGame.SpecialPickup_BoostOverride_TA` | `TAGame.SpecialPickup_Targeted_TA` |
| `TAGame.SpecialPickup_Football_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_GrapplingHook_TA` | `TAGame.SpecialPickup_Targeted_TA` |
| `TAGame.SpecialPickup_HauntedBallBeam_TA` | `TAGame.SpecialPickup_BallGravity_TA` |
| `TAGame.SpecialPickup_HitForce_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_Rugby_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_Spring_TA` | `TAGame.SpecialPickup_Targeted_TA` |
| `TAGame.SpecialPickup_Swapper_TA` | `TAGame.SpecialPickup_Targeted_TA` |
| `TAGame.SpecialPickup_Targeted_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.SpecialPickup_Tornado_TA` | `TAGame.SpecialPickup_TA` |
| `TAGame.VehiclePickup_Boost_TA` | `TAGame.VehiclePickup_TA` |
| `TAGame.VehiclePickup_TA` | `Engine.ReplicationInfo` |
| `TheWorld:PersistentLevel.VehiclePickup_Boost_TA` | `TAGame.VehiclePickup_Boost_TA` |

### C.5 Teams  (10)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.Teams.Team0` | `TAGame.Team_Soccar_TA` |
| `Archetypes.Teams.Team1` | `TAGame.Team_Soccar_TA` |
| `Archetypes.Teams.TeamWhite0` | `TAGame.Team_Freeplay_TA` |
| `Archetypes.Teams.TeamWhite1` | `TAGame.Team_Freeplay_TA` |
| `Engine.TeamInfo` | `Engine.Info` |
| `TAGame.GameEvent_Team_TA` | `TAGame.GameEvent_TA` |
| `TAGame.ProductAttribute_TeamEdition_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.Team_Freeplay_TA` | `TAGame.Team_Soccar_TA` |
| `TAGame.Team_Soccar_TA` | `TAGame.Team_TA` |
| `TAGame.Team_TA` | `Engine.TeamInfo` |

### C.6 Game events / rules / playlists  (51)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.GameEvent.GameEvent_Breakout` | `TAGame.GameEvent_Breakout_TA` |
| `Archetypes.GameEvent.GameEvent_FTE_Part1_Prime` | `TAGame.GameEvent_FTE_TA` |
| `Archetypes.GameEvent.GameEvent_Hockey` | `TAGame.GameEvent_Soccar_TA` |
| `Archetypes.GameEvent.GameEvent_HockeyPrivate` | `TAGame.GameEvent_SoccarPrivate_TA` |
| `Archetypes.GameEvent.GameEvent_HockeySplitscreen` | `TAGame.GameEvent_SoccarSplitscreen_TA` |
| `Archetypes.GameEvent.GameEvent_Items` | `TAGame.GameEvent_Soccar_TA` |
| `Archetypes.GameEvent.GameEvent_Season` | `TAGame.GameEvent_Season_TA` |
| `Archetypes.GameEvent.GameEvent_Season:CarArchetype` | `TAGame.Car_Season_TA` |
| `Archetypes.GameEvent.GameEvent_Soccar` | `TAGame.GameEvent_Soccar_TA` |
| `Archetypes.GameEvent.GameEvent_SoccarLan` | `TAGame.GameEvent_Soccar_TA` |
| `Archetypes.GameEvent.GameEvent_SoccarPrivate` | `TAGame.GameEvent_SoccarPrivate_TA` |
| `Archetypes.GameEvent.GameEvent_SoccarSplitscreen` | `TAGame.GameEvent_SoccarSplitscreen_TA` |
| `Archetypes.GameEvent.GameEvent_Tutorial_Advanced` | `TAGame.GameEvent_Tutorial_Advanced_TA` |
| `Archetypes.GameEvent.GameEvent_Tutorial_Basic` | `TAGame.GameEvent_Tutorial_Basic_TA` |
| `Archetypes.GameEvent.GameEvent_Tutorial_FreePlay` | `TAGame.GameEvent_Tutorial_FreePlay_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout` | `TAGame.GameEvent_KnockOut_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype` | `TAGame.Car_KnockOut_TA` |
| `Archetypes.KnockOut.GameEvent_Knockout:CarArchetype.StunlockArchetype` | `TAGame.Stunlock_TA` |
| `GameInfo_HeatseekerTerritory.GameInfo.GameInfo_HeatseekerTerritory:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_Hops.GameInfo.GameInfo_Hops:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_AprilFool.GameInfo.GameInfo_LTM_AprilFool:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_SpeedDemon.GameInfo.GameInfo_LTM_SpeedDemon:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_SpikeRush.GameInfo.GameInfo_LTM_SpikeRush:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_LTM_SuperCube.GameInfo.GameInfo_LTM_SuperCube:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_Possession.GameInfo.GameInfo_Possession:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_SnowDayTerritory.GameInfo.GameInfo_SnowDayTerritory:Archetype` | `TAGame.GameEvent_Territory_TA` |
| `GameInfo_Soccar.GameInfo.GameInfo_Soccar:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_SpikeDrop.GameInfo.GameInfo_SpikeDrop:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `GameInfo_Territory.GameInfo.GameInfo_Territory:Archetype` | `TAGame.GameEvent_Territory_TA` |
| `GameInfo_Tutorial.GameEvent.GameEvent_Tutorial_Aerial` | `TAGame.GameEvent_Training_Aerial_TA` |
| `GameInfo_Tutorial.GameEvent.GameEvent_Tutorial_Goalie` | `TAGame.GameEvent_Training_Goalie_TA` |
| `GameInfo_Tutorial.GameEvent.GameEvent_Tutorial_Striker` | `TAGame.GameEvent_Training_Striker_TA` |
| `Gameinfo_Hockey.GameInfo.Gameinfo_Hockey:Archetype` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_Breakout_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_FTE_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_KnockOut_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_Season_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_SoccarPrivate_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_SoccarSplitscreen_TA` | `TAGame.GameEvent_SoccarPrivate_TA` |
| `TAGame.GameEvent_Soccar_TA` | `TAGame.GameEvent_Team_TA` |
| `TAGame.GameEvent_TA` | `Engine.ReplicationInfo` |
| `TAGame.GameEvent_Territory_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.GameEvent_Training_Aerial_TA` | `TAGame.GameEvent_Training_TA` |
| `TAGame.GameEvent_Training_Goalie_TA` | `TAGame.GameEvent_Training_TA` |
| `TAGame.GameEvent_Training_Striker_TA` | `TAGame.GameEvent_Training_TA` |
| `TAGame.GameEvent_Training_TA` | `TAGame.GameEvent_Tutorial_TA` |
| `TAGame.GameEvent_Tutorial_Advanced_TA` | `TAGame.GameEvent_Tutorial_Basic_TA` |
| `TAGame.GameEvent_Tutorial_Basic_TA` | `TAGame.GameEvent_Tutorial_TA` |
| `TAGame.GameEvent_Tutorial_FreePlay_TA` | `TAGame.GameEvent_Tutorial_TA` |
| `TAGame.GameEvent_Tutorial_TA` | `TAGame.GameEvent_Soccar_TA` |
| `TAGame.Replay_Soccar_TA` | `TAGame.Replay_TA` |

### C.7 Player replication info (PRI)  (13)

| Archetype (spawn object) | Class |
|---|---|
| `Engine.PlayerReplicationInfo` | `Engine.ReplicationInfo` |
| `GameInfo_LTM_AprilFool.GameInfo.GameInfo_LTM_AprilFool:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `ProjectX.PRI_X` | `Engine.PlayerReplicationInfo` |
| `TAGame.Default__PRI_Breakout_TA` | `TAGame.PRI_Breakout_TA` |
| `TAGame.Default__PRI_KnockOut_TA` | `TAGame.PRI_KnockOut_TA` |
| `TAGame.Default__PRI_Possession_TA` | `TAGame.PRI_Possession_TA` |
| `TAGame.Default__PRI_TA` | `TAGame.PRI_TA` |
| `TAGame.PRI_Breakout_TA` | `TAGame.PRI_TA` |
| `TAGame.PRI_KnockOut_TA` | `TAGame.PRI_TA` |
| `TAGame.PRI_Possession_TA` | `TAGame.PRI_TA` |
| `TAGame.PRI_TA` | `ProjectX.PRI_X` |
| `TAGame.ProductAttribute_BlueprintCost_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_Blueprint_TA` | `TAGame.ProductAttribute_TA` |

### C.8 Mode actors / FX / misc  (75)

| Archetype (spawn object) | Class |
|---|---|
| `Archetypes.Misc.KeepUpIndicator` | `TAGame.KeepUpIndicator_TA` |
| `Archetypes.Mutators.SubRules.ItemsMode_RPS:DispenserArchetype.ItemPool.Obj` | `TAGame.SpecialPickup_BallCarSpring_TA` |
| `Archetypes.Mutators.SubRules.ItemsMode_RPS:DispenserArchetype.ItemPool.Obj_1` | `TAGame.SpecialPickup_BallCarSpring_TA` |
| `Archetypes.Mutators.SubRules.ItemsMode_RPS:DispenserArchetype.ItemPool.Obj_2` | `TAGame.SpecialPickup_BallFreeze_TA` |
| `Archetypes.Tutorial.Cannon` | `TAGame.Cannon_TA` |
| `Engine.Actor` | `Core.Object` |
| `Engine.GameReplicationInfo` | `Engine.ReplicationInfo` |
| `Engine.Info` | `Engine.Actor` |
| `Engine.NavigationPoint` | `Engine.Actor` |
| `Engine.Pawn` | `Engine.Actor` |
| `Engine.PlayerStart` | `Engine.NavigationPoint` |
| `Engine.ReplicatedActor_ORS` | `Engine.Actor` |
| `Engine.ReplicationInfo` | `Engine.Info` |
| `Engine.WorldInfo` | `Engine.ZoneInfo` |
| `Engine.ZoneInfo` | `Engine.Info` |
| `GameInfo_Breakout.GameInfo.GameInfo_Breakout:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_FTE.GameInfo.GameInfo_FTE:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_HeatseekerTerritory.GameInfo.GameInfo_HeatseekerTerritory:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Hops.GameInfo.GameInfo_Hops:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Items.GameInfo.GameInfo_Items:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_KnockOut.KnockOut.GameInfo_KnockOut:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_LTM_SpeedDemon.GameInfo.GameInfo_LTM_SpeedDemon:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_LTM_SpikeRush.GameInfo.GameInfo_LTM_SpikeRush:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_LTM_SuperCube.GameInfo.GameInfo_LTM_SuperCube:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Possession.GameInfo.GameInfo_Possession:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Season.GameInfo.GameInfo_Season:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_SnowDayTerritory.GameInfo.GameInfo_SnowDayTerritory:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_SpikeDrop.GameInfo.GameInfo_SpikeDrop:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Territory.GameInfo.GameInfo_Territory:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `GameInfo_Tutorial.GameInfo.GameInfo_Tutorial:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `Gameinfo_Hockey.GameInfo.Gameinfo_Hockey:GameReplicationInfoArchetype` | `TAGame.GRI_TA` |
| `ProjectX.Default__NetModeReplicator_X` | `ProjectX.NetModeReplicator_X` |
| `ProjectX.GRI_X` | `Engine.GameReplicationInfo` |
| `ProjectX.NetModeReplicator_X` | `Engine.ReplicationInfo` |
| `ProjectX.Pawn_X` | `Engine.Pawn` |
| `TAGame.BreakOutActor_Platform_TA` | `Engine.Actor` |
| `TAGame.CameraSettingsActor_TA` | `Engine.ReplicationInfo` |
| `TAGame.Cannon_TA` | `Engine.Actor` |
| `TAGame.Car_TA` | `TAGame.Vehicle_TA` |
| `TAGame.CrowdActor_TA` | `Engine.ReplicationInfo` |
| `TAGame.CrowdManager_TA` | `Engine.ReplicationInfo` |
| `TAGame.Default__CameraSettingsActor_TA` | `TAGame.CameraSettingsActor_TA` |
| `TAGame.Default__FreeplayCommands_TA` | `TAGame.FreeplayCommands_TA` |
| `TAGame.Default__MaxTimeWarningData_TA` | `TAGame.MaxTimeWarningData_TA` |
| `TAGame.Default__TrackerWallDynamicMeshActor_TA` | `TAGame.TrackerWallDynamicMeshActor_TA` |
| `TAGame.Default__ViralItemActor_TA` | `TAGame.ViralItemActor_TA` |
| `TAGame.Default__VoteActor_TA` | `TAGame.VoteActor_TA` |
| `TAGame.DynamicMeshActor_TA` | `Engine.Actor` |
| `TAGame.FreeplayCommands_TA` | `Engine.Actor` |
| `TAGame.GRI_TA` | `ProjectX.GRI_X` |
| `TAGame.InMapScoreboard_TA` | `Engine.Actor` |
| `TAGame.KeepUpIndicator_TA` | `Engine.Actor` |
| `TAGame.MaxTimeWarningData_TA` | `Engine.ReplicatedActor_ORS` |
| `TAGame.PlayerStart_Platform_TA` | `Engine.Actor` |
| `TAGame.ProductAttribute_Certified_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_NoNotify_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_Painted_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_Quality_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_SpecialEdition_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_TA` | `Core.Object` |
| `TAGame.ProductAttribute_TitleID_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.ProductAttribute_UserColor_TA` | `TAGame.ProductAttribute_TA` |
| `TAGame.RBActor_TA` | `ProjectX.Pawn_X` |
| `TAGame.Replay_TA` | `Core.Object` |
| `TAGame.SaveData_GameEditor_Training_TA` | `Core.Object` |
| `TAGame.Stunlock_TA` | `Engine.Actor` |
| `TAGame.TrackerWallDynamicMeshActor_TA` | `TAGame.DynamicMeshActor_TA` |
| `TAGame.TrainingEditorData_TA` | `Core.Object` |
| `TAGame.Vehicle_TA` | `TAGame.RBActor_TA` |
| `TAGame.ViralItemActor_TA` | `Engine.Actor` |
| `TheWorld:PersistentLevel.BreakOutActor_Platform_TA` | `TAGame.BreakOutActor_Platform_TA` |
| `TheWorld:PersistentLevel.CrowdActor_TA` | `TAGame.CrowdActor_TA` |
| `TheWorld:PersistentLevel.CrowdManager_TA` | `TAGame.CrowdManager_TA` |
| `TheWorld:PersistentLevel.InMapScoreboard_TA` | `TAGame.InMapScoreboard_TA` |
| `TheWorld:PersistentLevel.PlayerStart_Platform_TA` | `TAGame.PlayerStart_Platform_TA` |

# RLBot v5 Wiki — Reverse-Engineering Report (Sims, Bots, Physics)

- **Source:** <https://wiki.rlbot.org/v5/> — MkDocs-Material site, 33 pages, enumerated from `sitemap.xml` and read in full on 2026-10-07.
- **Scope:** everything on the wiki relating to the simulator/game surface, bot framework and wire protocol, and physics constants. Community-process pages are summarised only where they affect bot or sim behaviour.
- **Provenance rule:** every number below is *as stated by the wiki*. Items marked ⚠ are internally inconsistent or contradict this repo's calibrated values (§10). The wiki is a secondary source; RocketSim and our own captures are primary for physics (see ADR-0013, ADR-0019).

---

## 1. System overview

RLBot is a framework for **offline, scripted Rocket League bots** that drives the *real game* through an official Psyonix API (hence "safe to play with and against"). It is **not** a re-implementation of the physics; the simulator is Rocket League itself. v5 is **pre-release** (nearly feature-complete; language interfaces may still make breaking changes).

```
Match orchestrator (RLBotGUI / CleoPetra / any TCP client)
        │  MatchConfiguration
        ▼
RLBotServer.exe  (RLBot/core, C#, open source)
   ├── Bridge.dll (RLBot/bridge, CLOSED source, NDA'd Psyonix API shim — communication only)
   │        ▼
   │   Rocket League  (launched with `-rlbot`)
   └── TCP :23234 (FlatBuffers)  ◄──►  bot processes / script processes (any language)
```

Lifecycle: orchestrator sends `MatchConfiguration` → server starts Rocket League if needed (`-rlbot` flag) → server launches bot/script processes (`run_command`) → each connects, is briefed (match, index, field info), loads → server sends live data → each bot replies to each `GamePacket` with a `PlayerInput`.

### Why v5 (design rationale, verbatim themes)
| v4 problem | v5 fix |
|---|---|
| Closed-source C++ backend did more than API comms | Logic moved to open source; closed part is *only* API comms (Bridge.dll) |
| Backend in C++ | Rewritten in C# |
| DLL-based bot comms (legacy of DLL injection, 2018; Psyonix API from 2019) | WebSocket/TCP everywhere; typed interfaces |
| Match launching only in Python | Launch logic in RLBotServer; any language can start/stop/manage a match |
| Unused Psyonix API features | Integrated (Beginner bots, accolades, etc.) |
| Python slow | Backend faster; "significantly faster" overall |
| `.cfg` (no standard) | `.toml` |

**Platform support:** Windows (Steam or Epic); Linux (Proton/Steam, or Epic via Legendary/Heroic); macOS native dropped (use a VM). On Linux, if `run_command_linux` is absent, v5 tries `run_command` under Wine.

---

## 2. Wire protocol (RLBotServer socket spec)

- **Transport:** TCP. Default listen `0.0.0.0:23234` (IPv4+IPv6); override with first CLI arg (`RLBotServer.exe 12345`). Clients read `RLBOT_SERVER_IP` (default `127.0.0.1`) and `RLBOT_SERVER_PORT` (default `23234`). `RLBOT_AGENT_ID` is injected by the server into launched processes.
- **Framing:** `u16` big-endian length `n`, then `n` bytes of FlatBuffer. ⚠ A 16-bit prefix caps a packet at 65 535 B — this is the origin of the "Too many bytes sent this tick" rendering limit (§7.4).
- **Schema root:** `rlbot.fbs` in `RLBot/flatbuffers-schema` (`gamedata.fbs`, `matchconfig.fbs`, `rendering.fbs`, `comms.fbs`, `interfacepacket.fbs`, `corepacket.fbs`).
- **Direction naming (sender POV):**
  - `InterfacePacket` → union `InterfaceMessage` (client → server): `ConnectionSettings`, `InitComplete`, `PlayerInput`, `Ping`, `RenderingStatus`, match-comm/render/state-set messages.
  - `CorePacket` → union `CoreMessage` (server → client): `MatchConfiguration`, `FieldInfo`, `ControllableTeamInfo`, `GamePacket`, `BallPrediction`, `MatchComm`, `RenderingStatus`, `Ping`.
- **Handshake:**
  1. Send `ConnectionSettings` (`AgentId`; `CloseBetweenMatches` must be `true`; flags to disable ball prediction / match comms).
  2. Receive, in any order: `MatchConfiguration`, `FieldInfo`, `ControllableTeamInfo` (empty if `AgentId` invalid).
  3. From `ControllableTeamInfo` read `team`, `index`(es), `playerId`(s) — multiple for hiveminds. `team ∈ {0,1}` → `index` = position in `GamePacket.players`; `team = 2` → script, `index` = position in `MatchConfiguration.scripts`.
  4. Resolve own name via `identifier` (= `playerId`/`scriptId`). **Never use `index` for bots** — bot order in `MatchConfiguration` ≠ packet order.
  5. Heavy init (map is loading, match not started).
  6. Send `InitComplete` (empty) → server may start the match.
- **Main loop:** per tick `BallPrediction` always precedes `GamePacket`; `MatchComm` arrives between ticks; `RenderingStatus` pushed on permission change (client may request change; ignored if `DebugRendering.AlwaysOff`); `Ping` (either side) echoes a byte-string `cookie`.
- **Authorisation model:** agent ID selects which car(s) a connection may control; scripts' car inputs are ignored; extra read-only connections are **not** prevented — hence the explicit warning not to expose the server on untrusted networks (run in a VM, firewall it). Remote bots/scripts are **not** auto-started; set `auto_start_agents=false`.
- **Language interfaces:** official — Python, C#, Go, Rust, C++ (Java listed in repo links). Any language with TCP + FlatBuffers can implement the protocol; generate code by submoduling the schema repo and running `flatc`.

---

## 3. Game data model

| Message | Cadence | Content |
|---|---|---|
| `MatchConfiguration` | once, at start | participants, mutators, map, flags |
| `FieldInfo` | once | boost pad locations/sizes, goal locations (static map data) |
| `GamePacket` | **120 Hz** | balls, players, boost pad timers, scores, time, `match_phase` |
| `BallPrediction` | per tick, before `GamePacket` | 6 s of ball path (§4) |
| `PlayerInput` (controller) | as often as possible | buttons/axes; **previous input reused if a tick is missed** |
| `MatchComm` | between ticks | bot↔bot/script/human messaging (§8) |

### 3.1 v4 → v5 renames (authoritative list from the wiki)
`GameTickPacket→GamePacket`, `game_cars→players`, `game_boosts→boost_pads`, `game_info→match_info`, `ball→balls` (list; **multi-ball**), `num_*` removed (use `len`), `is_round_active / is_kickoff_pause / is_match_ended → match_phase` (`Active`/`Kickoff`/`Ended`), `has_wheel_contact→air_state` (`OnGround`), `spawn_id→player_id` (in `ControllableTeamInfo`), `MatchSettings→MatchConfiguration`, `FieldInfoPacket→FieldInfo`, `CollisionShape→shape` (direct union).

### 3.2 New per-player data in v5
last input; `has_double_jumped` and `has_dodged` as separate fields; `air_state ∈ {OnGround, Jumping, DoubleJumping, Dodging, InAir}`; `dodge_timeout`, `dodge_elapsed`, `dodge_dir`; `latest_touch` (time + which ball); `accolades` (previous-tick awards: EpicSave, Demolition, PoolShot, LowFive, HighFive, …). **Not available:** rumble item readout; Dropshot tile state and ball energy (lost with the Psyonix API).

### 3.3 Misc framework facts
- **Tick rate = `min(RL_FPS, 120)`** (physics runs at 120 Hz; VSync/FPS caps lower it). Non-standard tick rates were **removed** in v5; bots must be engineered to hold 120 Hz (use background threads for heavy work). In-game monitor `RLBot: N%` shows missed ticks; per-bot percentages show a slow bot; **>100 %** means two processes drive one car (stale bot from a prior match). `[Home]` toggles the stats overlay.
- Console commands via `set_game_state(console_commands=[…])`: `QueSaveReplay`, `Set WorldInfo WorldGravityZ <v>` (0 resets; don't spam per frame), `Set WorldInfo TimeDilation <v>`, `Stat FPS [0]`, `ShowDebug PHYSICS` (reveals `Set`-able class/attr names), `Pause` (toggle; bot output still called while paused), `ViewAutoCam`, `ViewDefault`, `ViewPlayer <team> <idx>`, `CycleHUD`, `CycleCamera`.
- **Steam-controller workaround (Epic installs):** launch options `-rlbot RLBot_ControllerURL=127.0.0.1:23233 RLBot_PacketSendRate=240 -nomovie -noeac`. ⇒ Rocket League itself exposes a **controller URL on 23233** and a **packet send rate** (240 here vs the 120 Hz physics tick) via command-line args; RLBotServer (23234) sits on top of that.

---

## 4. Ball path prediction

- 6 s horizon, **720 slices at 1/120 s**; each slice: `game_seconds`, `physics{location, velocity, angular_velocity}`. Ball rotation fields are always 0 (sphere).
- **Assumes no car contact**; "not exact" — refresh routinely. v5 algorithm was updated for accuracy across all ball-bearing modes.
- Related physics implementations the wiki points to: RocketSim (ZealanL), `rl_ball_sym` (VirxEC, Rust: mesh→triangles→BVH), Sam Mish's articles (§9).

---

## 5. Match configuration (sim control surface)

TOML files: `match.toml`, `bot.toml`/`*.bot.toml`, `script.toml`/`*.script.toml`, `loadout.toml`. JSON schemas at `rlbot.org/schemas/{agent,match}.json` (`#:schema` header; no loadout schema).

### 5.1 `[rlbot]` / `[match]`
- `launcher`: `Steam`(default) | `Epic` | `Custom` (`launcher_arg` = `legendary`|`heroic`) | `NoLaunch` (attach to a manually started RL). `auto_start_agents`, `wait_for_agents` (default true).
- `game_mode`: `Soccar`/`Soccer`(default), `Hoops`, `Dropshot`, `Hockey`, `Rumble`, `Heatseeker`, `Gridiron`, `Knockout` — affects ball prediction and some rules; modes can also be composed from mutators (reference configs in `python-interface/tests/gamemodes`).
- `game_map_upk` (e.g. `Stadium_P`, `UtopiaStadium_P`, `HoopsStadium_P`, `HoopsStreet_P`, `ShatterShot_P`). Custom maps load on Steam only; Epic = Psyonix maps only.
- `skip_replays`, `start_without_countdown`, `enable_rendering` (default **false**), `enable_state_setting` (default **true**), `auto_save_replay`, `freeplay` (use training/Bakkes features).
- `existing_match_behavior`: `Restart`(default) | `RestartIfDifferent` | `ContinueAndSpawn` (per-index diff: despawn changed cars, spawn new; **all** agent processes restarted; mutators untouched; used for LAN joins).

### 5.2 `[mutators]` (complete enum inventory)
| Key | Values (default first where stated) |
|---|---|
| `match_length` | FiveMinutes, TenMinutes, TwentyMinutes, Unlimited |
| `max_score` | Unlimited, OneGoal, Three…Hundred (1–100 goals) |
| `multi_ball` | One, Two, Four, Six |
| `overtime` | Unlimited, FiveMaxFirstScore, FiveMaxRandomTeam |
| `series_length` | Unlimited, 3/5/7 games |
| `game_speed` | Default, SloMo, TimeWarp |
| `ball_max_speed` | Default (6000), Slow, Fast, SuperFast |
| `ball_type` | Default, Cube, Puck, Basketball, Beachball, Anniversary, Haunted, Ekin, SpookyCube, Egg, PlayerSeeking, Dropshot, ScoreAbsorb, Shoe, PizzaPuck |
| `ball_weight` | Default, Light, Heavy, SuperLight, CurveBall, BeachBallCurve, MagnusFutBall, MagnusFutballLess |
| `ball_size` | Default, Small, Medium, Large, Gigantic |
| `ball_bounciness` | Default, Low, High, SuperHigh, LowishBounciness |
| `boost_amount` | NormalBoost, UnlimitedBoost, SlowRecharge, RapidRecharge, NoBoost |
| `boost_strength` | One, OneAndAHalf, Two, Five, Ten (+ Three per feature list) |
| `rumble` | Off, DefaultRumble, Slow, Civilized, DestructionDerby, SpringLoaded, SpikesOnly, SpikeRush, HauntedBallBeam, Tactical, BatmanRumble, GrapplingOnly, HaymakerOnly, SpikeRushForce, RPS |
| `gravity` | Default (650), Low (325), High (1137.5), SuperHigh (3250), Reverse |
| `ball_gravity` | Default, Low, High, SuperHigh (independent of car gravity) |
| `demolish` | Default, Disabled, FriendlyFire, OnContact, OnContactFF, OnBallContact, OnBallContactSilent, OnBallContactFF, OnBallContactFFSilent |
| `respawn_time` | ThreeSeconds, TwoSeconds, OneSecond, DisableGoalReset |
| `max_time` | Unlimited, ElevenMinutes (real-time cap; shots break ties) |
| `game_event` / `audio` | Default, Haunted (`game_event` also Rugby) |
| `territory` / `stale_ball` | Off/Territory; Unlimited/ThirtySeconds (ball teleported to other half) |
| `jump` | Default, Grounded, Two, Three, Four, Unlimited, NoJumps |
| `dodge_timer` | 1.25 s (default), 2 s, 3 s, Unlimited |
| `scoring_rule` | Default, Disabled (⚠ example file misspells `"Deafult"`) |
| `possession_score` | Off, 1/2/3 s |
| `demolish_score`, `normal/aerial/assist_goal_score` | point-value modifiers (Zero…Ten) |
| `input_restriction` | Default, Backwards |

### 5.3 Participants
- `[[cars]]`: `team` (Blue/0, Orange/1), `type` (`RLBot` | `Human` | `Psyonix`), `skill` (Beginner, Rookie, Pro, Allstar), `config_file`, `name`/`loadout_file` overrides, `auto_start`. An unnamed Psyonix bot is a random stock bot; a name matching a stock bot gets that bot's loadout.
- `[[scripts]]`: `config_file`, `auto_start`.
- `bot.toml [settings]`: `agent_id` (`author/bot-name[/version]`), `name`, `loadout_file`, `root_dir`, `run_command`, `run_command_linux`, `hivemind`, `logo_file` (400×300 PNG). `[details]` (GUI only): description, fun_fact, source_link, developer, language, tags (`1v1, teamplay, goalie, hoops, dropshot, snow-day, spike-rush, heatseeker, memebot`; `goalie` ⊥ `teamplay`).
- Loadouts: `[blue_loadout]`/`[orange_loadout]` with `team_color_id, custom_color_id, car_id (Octane=23 in the example), decal_id, wheels_id, boost_id, antenna_id, hat_id, paint_finish_id, custom_finish_id, engine_audio_id, trails_id, goal_explosion_id` + `.paint` sub-tables (`*_paint_id`: 0 None, 1 Crimson, 2 Lime, 3 Black, 4 Sky Blue, 5 Cobalt, 6 Burnt Sienna, 7 Forest Green, 8 Purple, 9 Pink, 10 Orange, 11 Grey, 12 Titanium White, 13 Saffron). Item IDs from a BakkesMod `dumpitems` CSV. Bots may generate/hot-swap loadouts at runtime (respawns the car; requires state setting). Keep a default `loadout.toml` as fallback for immediate-spawn modes.

---

## 6. State setting (the "sim hook")

`enable_state_setting` lets bots *and scripts* teleport/set cars and ball: **position, velocity, angular velocity, orientation, boost**, plus gravity/game speed/console commands. Enables repeatable scenarios mid-match, rewind snapshots, hover for orientation tuning, and gimmick physics ("meme bots"). Usually **disabled in tournaments**. (Per-language API is on the interface wikis, not this site.)

**Relevance to this repo:** this is exactly the injection primitive `tools/rb_tape_bot` + `rb_scenario` (initial state + input tape) rely on to capture mechanic ground truth; v5 also gives per-tick `PlayerInput` echo and `air_state`/dodge fields that could tighten capture labelling.

---

## 7. Bots, scripts, hiveminds, rendering

### 7.1 Scripts
`script.toml` processes: full game data, rendering and state setting, **no car control**; `team = 2` in `ControllableTeamInfo`; inputs ignored.

### 7.2 Hiveminds
`hivemind = true`. RLBot groups by **`agent_id` + team**: one process controls all same-team copies (the same bot on the other team is a separate process). Single-threaded (sequential) or multithreaded (not viable in Python — GIL). Messages from a hivemind must be sent from the `index` of the car carrying out the action.

### 7.3 Bot-making technique pages
- **Shooting at / away from a target** (language-agnostic): two targets (left/right post, ball-radius-adjusted e.g. `[±800, 5213, 321.3875]`); `direction_of_approach = clamp2D(car→ball, ball→left, ball→right)` (exploits RL's mirrored X axis); `offset_ball = ball − dir·92.75`; orbit via `final_target = offset + perp·(|angle(car→ball, dir)|·2560)`; required speed = distance/time, **infeasible if > 1410 (no boost) or > 2300 (boost)**. Anti-target = swap left/right.
- **Jump simulation** (see §9.3) and turn-radius / turning-speed models (§9.2).
- **ML FAQ:** RocketSim is the de-facto training sim; RLGym (rlgym-ppo, rocket-learn, rlgym-learn, GigaLearnCPP); replay→inputs via `RLCarInputSolver`, `carball`, Training Data Extractor; Ripple (behavioural cloning from a large replay dataset); supervised mimicry (Levi, TensorBot).

### 7.4 Rendering
Anchors (`RenderAnchor`: world and/or relative-to-car/ball with local offset; auto-follow, vanish if object destroyed), types `Line3D`, `PolyLine3D` (world only; genuinely one message in v5), `String2D/3D` (H/V align), `Rect2D/3D`. 2-D coords are 0..1 (top-left origin). Fixed 1-px lines; 3-D items don't scale with distance; font is monospace 10×20 px. Per-agent permission via `RenderingStatus`. Throttle: "Too many bytes sent this tick" → subsample (e.g., every 4th of 720 prediction slices), use `PolyLine3D`, and reuse a `group_id` (default `"default"`; renders persist until the same group id is re-sent).

### 7.5 MatchComms & TMCP
`MatchComm{index, team (0/1, 2=script), team_only, display, content(bytes)}`; `display` is for humans (quick-chat text; free-form in v5), `content` for machines; `team_only` is now enforced. v4 quick-chat IDs map to display strings (~60 presets, e.g. `Information_IGotIt → "I got it!"`). **TMCP 1.0** layers JSON on `content`: `{"tmcp_version":[1,0],"action":{type,…}}` with actions `BALL{time,direction}`, `BOOST{target}`, `DEMO{time,target}`, `READY{time}`, `DEFEND{}`; send only on change, ≤10 packets/s, team-only, refine time only if Δ>0.1 s; `-1` time / `[0,0,0]` direction = unknown.

### 7.6 Distribution & competition
RLBot Pack: open source, unique `agent_id`, `bob.toml` (Docker-built reproducible binaries; builders: python hardcoded/rlgym, rust, csharp, custom), submit via submodule PR, **skill cap ≈ Nexto** (if a bot scores 42 before Nexto's 28 it is too strong). Tournaments: fork rule, record matches, disclose borrowed code, tick-rate and hivemind policy declared up front, ≥24 h pre-deadline testing, run each bot on both teams. RocketHost = cloud dedicated servers (GUI-integrated); alternatives: LAN via Radmin + BakkesMod Rocket Plugin (port 7777), Parsec splitscreen; no console players.

---

## 8. Map extraction pipeline (collision geometry)

Reproducible recipe for getting the real arena collision mesh (directly relevant to ADR-0022/0025/0033/0037):
1. Windows, 64-bit; **32-bit** `umodel.exe` (`umodel_64` fails on some assets) with `-game=rocketleague -path=…\TAGame\CookedPCConsole`.
2. Maps are `*_P.upk` (e.g. `Stadium_P` = DFH Stadium).
3. `Export` → `UmodelExport/<map>/StaticMesh3/*.pskx`; collision files contain "collision": `Goal_STD_Collision`, `Field_STD_Collision_Corner`, `Field_STD_Collision_SideTop`, `Field_STD_Collision_SideBot` (4 for DFH Stadium = **all** field collision).
4. Blender (2.9+; tested 3.1) + `.psk` importer, import one file at a time, run the supplied script: dumps `*_ids.bin` (little-endian `i32` triangle indices) and `*_vertices.bin` (LE `f32` x,y,z triples; id = vertex order).
5. Consumer example: `rl_ball_sym` (Rust) builds mesh → triangles → BVH.

---

## 9. Physics reference (all values from the wiki)

### 9.1 World & coordinates
- Unreal units; **1 uu = 1 cm** (2778 uu/s ≈ 100 km/h). Scratch: divide by 32.
- **+Z up; +X is *left*; −Y points toward Blue's goal (team 0).** Yaw 0 at +X, increases clockwise; rotations are **YZX-ordered Euler**. (Mirrored X + clockwise yaw cancel so trig works normally.)
- Physics tick **120 Hz** (Δt = 1/120).
- Gravity **650 uu/s²** (Low 325, High 1137.5, SuperHigh 3250), supplied live as `world_gravity_z`.

### 9.2 Soccar arena
| Item | Value |
|---|---|
| Floor / ceiling | z = 0 / 2048 |
| Side wall / length | x = ±4096 / 7936 |
| Back wall / length | y = ±5120 / 5888 |
| Corner planes | meet axes at ±8064, 45°; length 1629.174 |
| Goal | height 642.775; centre-to-post 892.755; depth 880 |
| Wall-ramp radius | ≈256 (not truly circular) |
| Ball rest height | 93.15 |
| Car rest heights | Hybrid 17.00, Octane 17.01, Dominus 17.05, Breakout 18.33, Batmobile/Plank 18.65 |

**Boost:** 34 pads in RLBot order (list on wiki, mirror-symmetric; some per-map, use `FieldInfo`). Small ×28: +12 boost, 4 s respawn, cylinder r=144 h=165. Big ×6: +100, 10 s, r=208 h=168, at (±3584, 0, 73) and (±3072, ±4096, 73). Pickup when the car's **centre of mass** enters the cylinder (behaves differently when stationary).

**Kickoffs (Blue; Orange is 180° rotation):** (−2048,−2560) yaw ¼π; (2048,−2560) ¾π; (−256,−3840) ½π; (256,−3840) ½π; (0,−4608) ½π. **Respawn after demo:** (∓2304/∓2688, −4608) ½π.

### 9.3 Ball
- Radius **91.25** (⚠ other pages use 92.75 — see §10), mass 30 (arbitrary units), restitution **0.6**, max speed 6000, max angular speed **6 rad/s**.
- Drag ∝ velocity (linear): coefficient ≈ **0.030562**, terminal velocity ≈ 21 268.22 uu/s (10 634.11 at Low gravity). **Cars have no air drag.**

### 9.4 Car
| Quantity | Value |
|---|---|
| Max speed (boost) | 2300; supersonic ≥ 2200 |
| Max driven speed, no boost | 1410 |
| Mass | 180 (arbitrary) |
| Boost use | 33.3 /s |
| Boost accel | ground 991.666, air 1058.333 uu/s² |
| Braking (any amount) | −3500 uu/s² |
| Coasting (zero throttle) | −525 uu/s² |
| Throttle accel on ground | speed-dependent (smish notes) |
| Air throttle | +66.667 fwd / −33.334 rev uu/s² (⚠ this *is* a force while airborne) |
| Angular accel max | yaw 9.11, pitch 12.46, roll 38.34 rad/s² |
| Angular speed max | 5.5 rad/s |

**Curvature table** (`v` = forward speed): 0–500: `0.006900−5.84e-6·v`; 500–1000: `0.005610−3.26e-6·v`; 1000–1500: `0.004300−1.95e-6·v`; 1500–1750: `0.003025−1.1e-6·v`; 1750–2500: `0.001800−4e-7·v`; radius = 1/curvature. At full throttle+steer the car turns 90° in ≈0.775 s, 180° ≈1.55 s, 360° ≈3.1 s.

**Turning speed dynamics (throttle=1, steer=±1):** from rest, speed → 1234 asymptote: `v(t)=1234·(1−e^(−t/0.74704))` (≈5 s). From 2300, decays to 1234 over ≈7.5 s via a piecewise-linear fit (breakpoints at t = 0.1, 0.3, 1.3, 2.6, 3.2, 4, 4.5, 7.5 s; inverse function also given).

### 9.5 Jump model
- Gravity −650; **sticky force 325 uu/s²** toward the surface under the wheels for a short time after jumping — gone after **3 airborne ticks** (net −8.125 uu/s removed from jump velocity).
- **Jump impulse 292** (≈291.667) along car-up on press; identical for a non-flip second jump.
- **Hold bonus:** up to +292 additional along car-up over **0.2 s** (acceleration 1460 ≈ 292·5 uu/s², i.e. 12.17 uu/s per tick); minimum 3 ticks of bonus (+36.5).
- Second jump available **1.25–1.45 s** after first (the 0.2 s hold extension adds to the window).
- Max-speed rule: forces beyond 2300 are applied then the velocity is **renormalised to 2300**.
- Reference sim (`naive_double_jump_simulation`): semi-implicit Euler at 1/120, start z = 17.01, flat, zero velocity. ⚠ Uses float equality `simulation_time == 0`, applies the double-jump after `0.2+dt`, and ignores flip/dodge — "naive" by its own description.

### 9.6 Other modes
**Dropshot** (`ShatterShot_P`): 140 hex tiles in rows 7,8,9,10,11,12,13 | mid | 13…7; centre spacing 768 uu (256 across the middle, ~128 neutral strip); arena a rounded regular hexagon: centre→wall 4555, →corner 5026 (5259.66 unrounded), height 1986, floor z 3.2; ball radius **102.24**; ball charge 0/1/2 at **2500 / 11000** accumulated impact energy, minimum chargeable hit 500, absorption capped by `force_accum_recent` (≤2500, decays 2500/s); tile damage needs ≥250 uu/s normal velocity and ≥0.2 s since last damage; kickoff: ball launched straight up at 1000 uu/s, ~847 uu apex at 1.54 s; always spawn with 100 boost; boost regen ≈10/s after a 0.5 s delay and not while boosting; goals sit under the floor at (0, ±4430, −1040), w 5408, h 12640; own spawn table. Point-in-arena routine supplied.
**Hoops** (`HoopsStadium_P` Dunk House, `HoopsStreet_P` The Block): side wall ±2966.67, back wall ±3581, ceiling 1820, diagonals at ±5782, ramp radius 172; ball radius **98.38**, +1000 uu/s Z kick at kickoff; rings = semicircles at (0, ±2969, 364) radius 655, tube Ø≈42; 20 boost pads (6 big); own spawn table.

---

## 10. Cross-check against this repo & discrepancies

| Wiki value | rusty_bullet today | Verdict |
|---|---|---|
| gravity 650 | `world.rs` −650, `wheels.rs` STICKY_GRAVITY_Z | ✅ match |
| ball r 91.25, m 30, vmax 6000 | `BALL_COLLISION_RADIUS` 91.25, `BALL_MAX_SPEED` 6000 | ✅ |
| car m 180, vmax 2300, no-boost 1410 | `CAR_MASS`, `MAX_CAR_SPEED`, `UNBOOSTED_MAX_CAR_SPEED` | ✅ |
| brake 3500 / coast 525 | `BRAKE_DECELERATION` 3500; coasting factor noted | ✅ |
| boost use 33.3/s | `BOOST_USED_PER_SECOND` 33.3 | ✅ |
| jump 292, hold 1460/0.2 s | `drive/jump.rs` uses RocketSim's 291.667 / 1458.333 | ✅ (wiki rounds) |
| goal half-width 892.755 | `GOAL_HALF_WIDTH` | ✅ |
| boost accel ground 991.666 | repo keeps RocketSim's exact fraction (`boost.rs`) | ✅ rounding only |

**Wiki-internal inconsistencies / caveats (treat with care):**
1. **Ball radius:** 91.25 (Useful values) vs **92.75** (target-shooting tutorial) — 92.75 ≈ 91.25 + contact margin / RLBot-reported hitbox; ADR-0034 (calibrated car–ball contact radius) is the arbiter, not the wiki.
2. **Jump hold "+292 over 0.2 s"** vs acceleration 1460·0.2 = 292 ✅ consistent; but "minimum 3 ticks" contradicts a 0.2 s (24-tick) maximum only if read as the cap — it is a floor on tap duration.
3. **Boost-pad table:** two z values (0.082 vs 8.0) differ by pad size, but Hoops lists z = 0 and 8 — coordinates "have been rounded".
4. **Dropshot** corner distance: 5026 (rounded) vs 5259.66 (unrounded), and `TO_CORNER = 5260  # 4555 / sin(60)` in code (4555/0.866 = 5259.7) ✓.
5. **`boost_strength`**: mutator list omits `Three` but the v5 feature list includes 1×/1.5×/2×/3×/5×/10×.
6. **`scoring_rule = "Deafult"`** typo in the example match.toml would be rejected by a strict parser.
7. Wiki says ball "highly accurate" and "not exact" on adjacent pages; prediction ignores cars and doesn't model rotation.
8. Dropshot/Hoops tables (spawns, ring geometry) are single-source and uncalibrated; **not** covered by this repo's captures (Soccar only).

**Useful candidate follow-ups (not done here):** (a) cite §9.5/§9.4 turning fits as an independent check on `ADR-0011` curvature; (b) use `air_state`/`dodge_*` fields to label `rb_capture_ingest` captures; (c) mine v5 `PlayerInput` echo for tape-bot latency calibration; (d) mutator enums as a vocabulary for `rb_scenario` (gravity, ball size/weight/bounciness, jump, dodge timer, boost strength).

---

## 11. Page index (all 33 fetched)

Overview; `botmaking/` — ball-path-prediction, bot-loadouts, config-files, dropshot, game-data, hiveminds, hoops, jumping-physics, machine-learning-faq, manipulating-game-state, matchcomms, rendering, scripts, shooting-the-ball-towards-or-away-from-a-target, tick-rate, tmcp, useful-game-values; `community/` — community-guidelines, rlbot-pack, story-mode (v5 planned, post-beta), tips-for-running-tournaments; `framework/` — architecture, console-commands, operating-system-support, remote-rlbot, sockets-specification, supported-languages, v5; `miscellaneous/` — extracting-map-meshes, lan-setup, rocket-host, steam-controller-settings.

**Not covered by the wiki (gaps):** the FlatBuffers schema bodies (fields live in `RLBot/flatbuffers-schema`), the `PlayerInput`/controller field list, state-setting API, car hitbox dimensions and per-body offsets (only a link to a OneDrive spreadsheet), contact/collision solver detail, demolition rules, flip/dodge impulse math, and the Psyonix bot skill models. Those need the schema repo, RocketSim, or captures.

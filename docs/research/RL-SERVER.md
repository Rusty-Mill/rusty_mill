# Rocket League server: research note

Status: **research only, no code.** Written 2026-10-11.

`rusty_bullet` began as a from-scratch Rocket League server, to take the netcode apart and
diagnose lag, then drifted into the simulator. This note collects what is publicly known about
the netcode, what a server clone needs beyond the simulator, how to stay legally clean, and how
it builds on the simulator and the RLBot port. It ends with a phased roadmap and the first
experiment. Paths are under `crates/apps/rocket_league/` unless stated.

Most of the groundwork is already in `rusty_bullet`: RB-RESEARCH-S001 (the GDC talk), ADR-0001
(server-authoritative with client prediction), RB-NET-001 (a placeholder spec), RB-RESEARCH-O002
(a legal review of binary reverse engineering) and the Phase 3 entry in `docs/roadmap/ROADMAP.md`.
This note re-reads the primary source, corrects one claim, and adds the missing parts.

## 1. What is publicly known

### Source and how it was checked

The one primary source is Jared Cone's GDC 2018 talk, "It IS Rocket Science! The Physics and
Networking of Rocket League"
([slides](https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf), 182 pages;
[video](https://www.youtube.com/watch?v=ueEmiDM94IE)). For this note the slide text was
extracted and read. The spoken talk was not watched, so anything the speaker said that is not on a
slide is absent here. Statements below are marked **slides** (read in the extracted text),
**secondary** (a forum post or summary, not verified) or **ours** (our own design inference).

### Findings

| Topic | Finding | Basis |
|---|---|---|
| Physics engine | Bullet, chosen as open source (debuggable, modifiable); single-threaded; discrete collision detection; about a week to integrate | slides |
| Tick rate | Fixed, **120 Hz (8.33 ms)**. A slower rate means larger steps, larger penetrations and inconsistent hits; a higher rate is more consistent but costs more, "especially for network corrections" | slides |
| Authority | "100% server authoritative"; input delay "is not an option"; the client predicts rigid-body vehicles | slides |
| Rejected design 1 | Server waits for client input before stepping. Input jitter and loss then stall the simulation, and hitting a moving object can desync | slides |
| Rejected design 2 | Server-side lag compensation (FPS style: client predicts the hit, server rewinds to confirm). The final design has "no server-side lag compensation" | slides |
| Ball and cars | The client predicts its own car; the ball is server-authoritative and the client interpolates it, so the car and the ball live "in two different timelines". The final answer: the client predicts everything, drives to where the ball is going to be, which works well for the ball (predictable) and worse for other cars (unpredictable) | slides |
| Input path | The client sends an input for every physics frame; they arrive at irregular rates; the server buffers them and never pauses. This also removes some cheats (speed, jitter) at the cost of added average latency | slides |
| Buffer control | Avoid an empty buffer (physics then runs on the previous input) and a large one (adds latency). **Upstream throttle:** the server tells the client to run extra or fewer physics frames. **Downstream throttle:** the server consumes 0, 1 or 2 inputs per frame (use one input for two frames when low; two inputs for one frame when full), "effective but with minor desyncs" | slides |
| Correction loop | Client records input and frame number, runs physics, keeps a history, sends input and frame number. The server reads the input, runs physics, and returns the client's frame number plus the **physics state**. The client compares it to its recorded history; a large difference "requires correction": update history, revert all physics actors to that frame, run multiple physics frames to catch up | slides |
| Correction cost | "200ms ping, 120hz = 24 correction frames" | slides |
| Results claimed | No input delay; high-ping clients do not ruin the game; moving objects are reliably hit; full server authority | slides |
| Server send rate | A Steam forum post quoting a Psyonix network developer says physics runs at 120 Hz on servers and updates go out at about 60 per second. Not on the slides | secondary, unverified |
| Input buffer telemetry | The in-game performance graphs include an input-buffer counter (IBuf), the number of client input frames the server has queued ([Epic help page](https://www.epicgames.com/help/c-37599050/c-Trending_0/rocket-league-performans-ve-ag-grafikleri-yalnizca-pc-a11386071)) | secondary |

### What the slides do not say

Snapshot rate, what exactly is in a snapshot beyond "physics state", compression and delta
coding, the correction threshold, how a correction is shown on screen (blended or snapped), and
what is replicated for match state (score, clock, boost pads). These are open and must not be
asserted.

### A claim in this repository that the slides do not support

RB-RESEARCH-S001 and ADR-0001 say periodic snapshot corrections are sent "rather than continuous
full state" and that visible rubber-banding is "almost certainly reconciliation smoothing, not raw
latency". The slide text contains the correction loop and its cost but neither a statement about
periodic versus continuous state nor any mention of smoothing. Either those statements come from
the spoken talk or they are inference. Before ADR-0001's reasoning is relied on for the lag
diagnosis, the video should be checked for them, or both should be relabelled as hypotheses.

## 2. What a server clone needs beyond the simulator

What exists: `rb_physics_bullet` (`PhysicsWorld`, which derives `Clone`), `rb_env` (`Env` steps at
`TICK_SECS = 1/120`, with match flow, boost pads, kickoff and replay), `rb_domain::PhysicsFrame`,
and the RLBot client and bridge. What is missing:

| Piece | What it is | Notes |
|---|---|---|
| Transport | Own UDP-style protocol with a reliable side channel; connect, keepalive, disconnect | `std::net` only; no new dependency needed. Own wire format, not the real game's |
| Input channel | One input per tick tagged with a frame number | Repeating the last few inputs in each packet to survive loss is ours, not from the slides |
| Server input buffer | Per-client queue with a target depth and the two throttles | Straight from the slides; the depth policy is open |
| Snapshot and replication | Frame number plus car, ball, pad and match state to each client; reliable events for goals and demolitions | Format, rate and delta coding are ours to choose |
| Client prediction | Frame-numbered history, compare, revert, resimulate | Needs a state snapshot and restore. `PhysicsWorld` is `Clone`; `Env` is not (it also holds match flow, pad and respawn state), so a full-state snapshot is new work |
| Ball handling | Client predicts the ball too, per the slides' final design | Interpolation was the rejected first attempt |
| Correction display | How a correction looks on screen | Not in the slides; our design |
| Session layer | Lobby, join by address, match lifecycle, spectators, bots as clients | Match flow already exists in `Env`; the lobby and lifecycle around it do not |
| Matchmaking | A stub only: list or join by address, no ranking | Out of scope beyond that |
| Anti-desync | Frame numbers, state-hash checks, a correction threshold | Ours. A correction is the recovery path, so hashing is for diagnostics |
| Network conditioner | In-process link with latency, jitter, loss and a seeded random source | Needed for repeatable lag experiments; RB-NET-001 already names it as an open question |
| Instrumentation | Corrections per minute and their size, resimulated frames, input-buffer depth | The diagnostic payoff. Ties to the sim viewer cards (correction overlays) |

On determinism: reconciliation does not need bit-identical client and server (it resimulates from
the server's state). Two copies of the same `PhysicsWorld` agree with each other whether or not
either matches the real game, so a loopback client and server can only disagree because their
inputs, histories or states differ, never because of simulator fidelity. Netcode correctness
(prediction, buffering, reconciliation) and simulator fidelity (does it match the real game) are
separate questions with separate tests; see sections 4 and 6. RB-SIM-001 only aims at run-to-run
determinism so far.

## 3. Legal and ToS constraints

Not legal advice; the owner's counsel decides.

Existing findings (RB-RESEARCH-O002, RB-RESEARCH-S005):
- Psyonix's EULA redirects to Epic's, which contractually prohibits reverse engineering,
  decompiling and disassembling the software. The Rocket League Code of Conduct separately
  prohibits exposing unreleased features found in the game's code.
- RB-RESEARCH-S005 records that Easy Anti-Cheat blocks BakkesMod in online matches, so inputs
  captured through that route are offline-only. That is the one capture route inspected; it does not
  establish that every possible online capture method is impossible.
- The binary-RE question is still open, and practical work has not started and is blocked.

Postures for a server clone, and a recommendation:

| Posture | What it means | Assessment |
|---|---|---|
| **A. Spec-only from public sources** | Build from the GDC talk, Bullet (zlib), RocketSim's open source and our own offline captures. Our own protocol, compatible with the real game only at gameplay level (same rules and physics constants), never on the wire | **Recommended** as the lowest-risk of the three. It is what the project already does. It is not a clearance: see "Existing game-derived material" below |
| B. Formal clean-room | One person reads a protected source and writes a spec, another implements | Not needed if nothing protected is read; heavy for one person |
| C. Reverse-engineer the real protocol from captured traffic or the binary | Would give wire compatibility with official clients or servers | **Not recommended.** Contract breach risk, EAC, account risk, and the earlier decision not to |

Practical rules under A: do not capture official-server traffic; do not read or decompile game
binaries for protocol detail; do not accept official clients or connect to official servers; do
not use Psyonix art or names; add no new game-derived data without the review described below;
keep the physics constants sourced from captures and open-source reimplementations, as ADR-0013
and ADR-0019 describe (the capture is ground truth).

### Existing game-derived material

The repository already ships game-derived data, so the posture above reduces new exposure; it does
not remove existing exposure. `arena::standard_meshes` loads 16 collision mesh files
(`crates/rb_physics_bullet/assets/soccar/mesh_0.cmf` to `mesh_15.cmf`), copied unmodified from the
`rlgym_rocket_league` 2.0.1 distribution, which redistributes RocketSim's files under Apache-2.0.
`rusty_bullet/THIRD_PARTY_NOTICES.md` (lines 81 to 94) states that the geometry itself is Rocket
League's, dumped from the game by RocketSim's authors, and that Rocket League is a Psyonix / Epic
trademark; ADR-0037 records the choice. Whether and how this affects licensing or any release is
an open question for the owner's counsel. This note does not conclude that anything infringes and
does not ask for anything to be removed or relicensed.

## 4. How it builds on the simulator and the RLBot port

- **The simulator** is the core: the same `PhysicsWorld` runs on both sides, which is the central
  fact of the GDC design. Its open fidelity gaps (pogo and boost errors, dodge fidelity) do not show
  up as client and server disagreement, because both sides share them. They matter for
  diagnostics: comparing correction behaviour with the real game assumes the simulator resembles
  it. Fidelity is checked separately, against real-game captures with `rb_verify`.
- **The RLBot port** gives drivers: bots (`rb_rlbot_client`, `rb_rlbot_bridge`, the chaser) can be
  the clients of a test server, so most of the netcode can be exercised with no human and no game.
- **A sim-backed RLBot-compatible server** (RESEARCH-BACKLOG O007, reading B) is the stepping stone:
  a loop that steps the world from bots' inputs is the same loop a network server runs, with the
  network removed. Build that loop once, put the buffer and transport in front of it.
- **The viewer and recorded runs** (the sim viewer cards) show a recording and, later, correction
  events beside it. The input timeline there is the same stream as the input channel here.

## 5. Phased roadmap

Phases line up with `PHASE-3-NETCODE` in the roadmap, whose exit criterion is: our own client and
server pair reproduces the reconciliation behaviour from the GDC talk under simulated network
conditions.

1. **Foundations (small).** Two `Env` instances fed identical inputs stay bit-identical (a control,
   not a fidelity test); a
   full-state snapshot and restore for `Env` (cloning `PhysicsWorld` plus flow, pad and respawn
   state); decide the packet and frame-number conventions. Exit: a determinism test and a
   restore test.
2. **Loopback harness.** In-process server and client with a network conditioner, input buffer,
   prediction and reconciliation, and the metrics above. Exit: the first experiment below runs
   and reports. This is where RB-NET-001 gets its real design.
3. **Real transport.** UDP over localhost then LAN, handshake, snapshot format, reliable events.
4. **Sessions.** Lobby, match lifecycle, several clients, bots and spectators as clients.
5. **Playable client.** Rendering by reusing the replay viewer or a separate decision; play
   against a friend. Meets the roadmap's phase 3 acceptance wording.
6. **Diagnostics.** Answer the original question: what correction sizes and rates realistic
   jitter produces, and whether they match what you see in the real game. There is no official
   ground truth to compare with, so this stays a hypothesis test, not a verification.

## 6. First experiment: loopback reconciliation

The experiment that teaches the most for the least effort.

- **Setup:** one process; two `Env` instances, one as server and one as predicting client, from the
  same start. A scripted input source (the chaser or a recorded tape). A link between them with
  configurable latency, jitter and loss, and a seeded random source so runs repeat.
- **Step zero (control):** confirm the two instances stay bit-identical on a perfect link (no
  latency, no jitter, no loss) with identical inputs. If they do not, that is the first finding.
  With identical copies this also means no correction should ever fire, so it proves nothing about
  reconciliation by itself.
- **Exercise reconciliation on purpose:** give the client a deliberately different history from
  the server: a small offset in the starting state, dropped, delayed and reordered inputs, a
  forced input-buffer underrun, an extra input only the client sees. Each should produce a
  correction of a predictable size.
- **Then measure** per run: corrections per minute, frames resimulated per correction, position
  error at correction time, and input-buffer depth, as latency, jitter and loss vary; and the same
  with the buffer's two throttles on and off.
- **What it teaches:** which netcode situations cause corrections (buffer underruns, ball hits on
  a predicted car, loss) and how large they are; how steeply the cost grows with latency
  (the slides give 24 frames at 200 ms and 120 Hz as the reference point); and whether the buffer
  throttles introduce the "minor desyncs" the slides mention. It cannot reveal simulator fidelity
  error: that error is the same on both sides, so fidelity stays a separate track (real-game
  captures and `rb_verify`). It needs no new dependency, no game and no protocol work.

## Open questions

1. Are the "periodic snapshots" and "reconciliation smoothing" statements in the spoken talk
   (section 1)? Check the video.
2. What is the real server send rate? The only source is an unverified forum quote.
3. Snapshot and restore for `Env`: is a clone of `PhysicsWorld` plus the flow state enough, or
   is there hidden per-car state (wheel contact, dodge, respawn) that needs its own capture?
4. Does the server clone live in `rusty_bullet` or as a new crate in the Rocket League family?
5. The unimported branch `claude/rocket-league-server-clone-u74q45` (49 commits, +82k lines) holds
   the earlier server work. It was not read for this note; it should be before any roadmap phase
   is started, to avoid redoing it.

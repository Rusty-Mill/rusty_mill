# ADR-0059: RLBot v5 and BakkesMod setup for tape-bot captures

- Status: Proposed (run sequence verified against docs and source, not yet
  against the game)
- Date: 2026-10-06
- Deciders: baileyrd
- Related: RB-RESEARCH-O005, ADR-0056, ADR-0057, ADR-0058,
  `docs/research/BOT-RUN-SHEET.md`, `tools/rb_tape_bot/README.md`
- Supersedes/Superseded by: none

## Context

`tools/rb_tape_bot` replays a scripted input tape in the real game so the
BakkesMod plugin `rusty_bullet_capture` can record repeatable mechanics.
Running it needs RLBot v5 and BakkesMod attached to the same Rocket League
process, a way to tell the bot which scenario to play, and a crate that
speaks the installed core's protocol. The README's run steps were written
before anything was installed; the first run session (2026-10-06) checked
them against the current RLBot docs and source (`RLBot/core` master =
v5.0.0-rc17, `RLBot/gui` beta23, `RLBot/launcher`, `RLBot/wiki`
`docs/v5`, `RLBot/rust-interface` 0.6.0). Installing RLBot in that session
was refused by the tool permission policy, so the choices below are made
from documentation and source, and the run itself is still owed.

## Decision drivers

- One documented, repeatable sequence the owner can run alone.
- Offline only; no change to how BakkesMod or the game are installed.
- Minimal change to the bot: configuration, not code, where possible.
- No dependency bump without a demonstrated need.

## Considered options

1. **Launch order.** (a) Start the game with BakkesMod, then start the
   RLBot match. (b) Start BakkesMod first, let RLBot core launch the game.
   Core's `LaunchManager` kills any running Rocket League and relaunches
   it with `-rlbot ... RLBot_PacketSendRate=240 -nomovie` (for Epic, after
   reading the login arguments from `Launch.log`), so (a) is only (b)
   with an extra game start. The wiki's LAN guide documents (b) for
   BakkesMod plugins, and the match option `freeplay` is described as
   allowing "Bakkesmod plugins".
2. **Passing the scenario path.** (a) Export `RB_TAPE` in the shell that
   starts the GUI and rely on inheritance through launcher, GUI, core and
   `cmd /c`. (b) The `[settings.environment]` table in `bot.toml`, which
   core reads (`ConfigParser.GetEnvironment`) and applies to the bot
   process after the inherited environment. (c) One `*.bot.toml` per
   scenario. (d) Change the bot to read a config file or argument.
3. **Crate version.** Keep `rlbot` 0.6.0 or bump. 0.6.0 (2026-09-21) is
   the newest release; its schema (`c38374e`) differs from core rc17's
   (`f90c844`) only in comments and a `deprecated` attribute.
4. **Match start.** The GUI, or a small program sending
   `MatchConfiguration` to core's socket as the crate's `start_match`
   example does.

## Decision

- Launch order (b): BakkesMod running first, then start the match from the
  RLBot GUI with **Freeplay** and **Enable State Setting** ticked, Epic as
  the launcher; core relaunches the game and BakkesMod injects into it.
- Scenario path (b): `RB_TAPE` lives in `bot.toml`'s
  `[settings.environment]`, set to `scenarios\prompt_dodge.json`, edited
  per run. It is the mechanism core documents, needs no bot code change,
  and is explicit in the repository. (c) is the fallback if editing per
  run proves annoying; (d) is not needed.
- Keep `rlbot` 0.6.0.
- Match start through the GUI for now; the scripted start is noted in the
  README and built only if a second session wants it.
- Install path: the official MSI from `RLBot/launcher` (per-user, no
  elevation, installs `launcher.exe` to `%LOCALAPPDATA%\RLBot5\bin`),
  which then fetches `rlbotgui.exe` and `RLBotServer.exe`. The owner runs
  it by hand.

## Consequences

### Positive

- The README's run steps now match the current v5 install path and core's
  actual behaviour, with the source locations cited.
- Two of the plan's open risks (state setting from a bot, protocol
  mismatch) are closed from source without a run.
- No new dependency, no bot code change.

### Negative / tradeoffs

- The decisive risk (BakkesMod recording while core owns the game) is
  still only supported by documentation, not observed.
- `bot.toml` is edited per scenario, which dirties the working tree
  during a session; acceptable for a tool, and a per-scenario
  `*.bot.toml` set is the easy fix.
- Core's `RLBot_PacketSendRate=240` against the wiki's 120 Hz cap leaves
  the tape's one-packet-per-tick assumption to be measured in Stage 1.

## Validation and revisit triggers

- Validate: Stage 1 of `BOT-RUN-SHEET.md` (prompt_dodge captured, first
  frame within 5 uu of the set location, lag 0 or 1, no drift). Record the
  numbers in the run sheet and change this ADR's status to Accepted.
- Revisit if BakkesMod does not inject into the core-launched process
  (then try injecting after the match starts, or `launcher = NoLaunch`
  with the game already running under BakkesMod), if packets arrive at
  240 per second (key the tape on `match_info.frame_num`), or if a core
  release changes the schema (`RLBotServer.exe --version`, compare with
  `rlbot::RLBOT_FLATBUFFERS_SCHEMA_REV`).

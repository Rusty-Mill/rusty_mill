# rusty_bullet_capture (BakkesMod plugin)

Implements `RB-VERIFY-002-FR-001`: records a real Rocket League session
(freeplay or offline/local match — BakkesMod is blocked online by Easy
Anti-Cheat, so this is offline-only by construction) to the JSON-Lines
capture format defined by
[ADR-0005](../../docs/adr/0005-capture-file-format-and-input-schema.md), so
`rb_capture_ingest`/`rb_verify_cli` can score this physics-core port against
a real recording instead of only replays.

**Status: source written, not yet built or run.** This is a Windows +
BakkesMod + Rocket League deliverable — none of that exists in the sandbox
this was written in, so nothing here has been compiled, loaded, or verified
against the real game. Treat it as a draft to build and shake out on your
own machine, not as a finished, tested tool. If the actual hookable event
name (`Function TAGame.Car_TA.SetVehicleInput`, see below) turns out to be
wrong or renamed in a current Rocket League build, the fix is a one-line
change to the string in `RustyBulletCapturePlugin.cpp`'s `onLoad()`.

## What it does

- Hooks `Function TAGame.Car_TA.SetVehicleInput` (post), which fires once
  per car per physics tick. Since a tick can fire this once per car, the
  plugin dedupes on the ball's own `GetPhysicsFrame()` counter so exactly
  one capture line is written per tick regardless of car count.
- Each line is the ball's transform/velocity plus every car's transform,
  velocity, boost amount (converted from BakkesMod's 0.0-1.0 fraction to
  this project's 0-100 scale, matching `rb_replay_ingest`'s convention),
  and full controller input (including analog pitch/yaw/roll, which a
  replay can never recover — see ADR-0005 for why that matters).
- `player_id` is just this recording session's PRI iteration order (0, 1,
  2, ...), not a stable cross-session id. That's deliberate: per
  `RB-VERIFY-002`'s own Non-goals, this is a one-off capture script for
  this pipeline, not a general plugin platform, and a single recording
  session never needs to reconcile player identity against a second one.

## Building

You need Visual Studio's C++ toolchain (MSVC) and a copy of the
[BakkesModSDK](https://github.com/bakkesmodorg/BakkesModSDK) — either your
existing BakkesMod install's own copy
(`%appdata%\bakkesmod\bakkesmod\bakkesmodsdk\`) or a fresh clone.

With CMake (the SDK's own recommended
[plugin template](https://github.com/Martinii89/BakkesmodPluginTemplate)
uses the same approach):

```powershell
cmake -B build -DBAKKESMOD_SDK_PATH="C:\path\to\BakkesModSDK"
cmake --build build --config Release
```

This produces `rusty_bullet_capture.dll`. Without CMake, the SDK's own
README documents a one-line manual build from an "x64 Native Tools Command
Prompt for VS 2019 (or later)":

```powershell
cl /LD /std:c++17 -I <sdk>\include RustyBulletCapturePlugin.cpp <sdk>\lib\pluginsdk.lib /Fe:rusty_bullet_capture.dll
```

## Loading and using it

1. Copy `rusty_bullet_capture.dll` into BakkesMod's `plugins` folder
   (`%appdata%\bakkesmod\bakkesmod\plugins\`).
2. In-game, open the BakkesMod console (default: F6) and run:
   ```
   plugin load rusty_bullet_capture
   rb_capture_start my_session.jsonl
   ```
   Drive around (freeplay is simplest), then:
   ```
   rb_capture_stop
   ```
3. `my_session.jsonl` is written relative to Rocket League's own working
   directory unless you pass an absolute path to `rb_capture_start`. Feed
   it to `rb_verify_cli` the same way as the vendored replay fixture.

## Known limitations (by design, not bugs)

- Not usable for online-match ground truth (Easy Anti-Cheat blocks
  BakkesMod online) — see `RB-VERIFY-002`'s Non-goals.
- No format-version field in the output, matching ADR-0005/`RB-RESEARCH-O003`'s
  "default to the smaller option" resolution.
- `player_id` is a per-session ordinal, not a stable identity (see above).

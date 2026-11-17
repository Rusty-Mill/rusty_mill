# #1166 Second paid lens run: task brief (SSH host launcher)

Status: brief only, saved 2026-11-17 for the #1166 smoke-run plan card. Per Nano's Q50 answer (relayed 2026-11-17) it is the intent for the **second** paid run; the first run (the smoke run) uses the toy `sweep v0.1-smoke` brief in `1166-smoke-run.md`, unchanged. This file changes nothing in that plan.
Source: pasted by the owner in the Claude Code session of 2026-11-17, with Nano's approval of one paid lens run that day (relayed, voice, claude.ai chat). Copy it to the second run's own task dir `brief.md` when that run is opened; do not overwrite the toy `/srv/lel/toy/brief.md`.

## Intent (the brief, verbatim)

rusty_mux gets an SSH host launcher, modelled on Orca's Remote Hosts panel.

What it does
- Reads the user's SSH config (on Windows, `%USERPROFILE%\.ssh\config`) and lists its named hosts.
- Picking a host opens a new tab running an ssh session to that alias.
- Shows which listed hosts have a live session opened this way.

Done looks like
- I open the list, pick a host, and I'm in a shell on it in a new tab, on Windows and Linux.
- The SSH config is read-only. Nothing is written to it.
- A missing or malformed config gives a clear message, not a crash.

Out of scope
- Editing or managing hosts.
- Orca's other status-bar items: usage meters, keep-awake, resource manager, ports.
- Any remote-server daemon. Plain ssh only.

## Questions for the lenses
- Smallest surface: a new action in the existing command palette, or a status-bar panel?
- How much of the config to honour: Include files, wildcard Host patterns, Match blocks?
- Hand-rolled parser or a dependency? Default is no new dependency.

## Run rules (from the owner)
- Same limits as the first run: `--max-passes 5`, default call cap. Dollar caps do not enforce in v0.
- Paid providers: codex; claude only once its `no_overage` entry exists.
- Output is a board of claims only: no cards, no code.

## Open
- The brief has no "Never" or "Irreversible" lines as the toy brief does. Nothing here is irreversible; the lenses will say so or not.
- Its work area is `rusty_mux` (expert: `rusty-term-expert`), not Let Em Loose.

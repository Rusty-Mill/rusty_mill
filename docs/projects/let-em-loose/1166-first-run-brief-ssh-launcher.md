# #1166 First paid lens run: task brief (SSH host launcher)

Status: brief only, saved 2026-11-17 for the #1166 smoke-run plan card. It is the intent for the first run. It changes nothing in `1166-smoke-run.md`, whose toy brief (`sweep v0.1-smoke`) still stands for the local-only smoke run (D134). Whether this brief replaces the toy brief for the first run is for Nano to say (see "Open" below).
Source: pasted by the owner in the Claude Code session of 2026-11-17, with Nano's approval of one paid lens run that day (relayed, voice, claude.ai chat). Copy it to `brief.md` of the task dir (`/srv/lel/toy/brief.md` in the 1166 layout) when the run is opened.

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
- One live run, `--max-passes 5`, default call cap. Dollar caps do not enforce in v0.
- Output is a board of claims only: no cards, no code.

## Open
- Brief fits `bbp open --mode claims` as written, but it has no "Never" or "Irreversible" lines as the toy brief does. Nothing here is irreversible; the lenses will say so or not.
- Its work area is `rusty_mux` (expert: `rusty-term-expert`), not Let Em Loose.

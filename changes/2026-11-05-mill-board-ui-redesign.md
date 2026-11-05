---
category: Changed
changelog: Mill Board web UI redesigned: scope bar, colour strip, job-ticket cards, approvals that wait on you.
---
## 2026-11-05 - Mill Board web UI redesign

- **Changed:** the Mill Board web UI is redesigned as a press room's job board. A graphite top bar holds the views as
  tabs, a quick add and who you are (fixed when signed in). The side panel is replaced by a scope bar: a repository
  picker, project chips and agent chips. A colour strip shows each column's open cards and jumps to it; empty columns
  fold to a spine; cards are job tickets (number, priority mark, title, project, holder, age) with extra tags folded
  to a count. Cards at a human gate or with an open decision are marked "Waits on you", in magenta, with Approve or
  Answer. Metrics is a grid of panels: a weekly added-against-shipped chart, open cards by column, priority, agent
  and project, and the cards idle longest. Settings is a full page with a section rail, each section with a short
  description. Roadmap is a timeline: a row per project (or, with one project chosen, per card from Ready on, with
  shipped cards as diamonds), each bar running from when its work began and filled with the share shipped; under it a
  progress ring with In progress, Up next, Blocked and Waits on you, recent activity, then the lanes. Archive,
  Decisions and the card dialog share the look; dark mode follows the OS.
- **Added:** the UI's two typefaces (SIL OFL 1.1) are served from the binary at `/ui/fonts/`.
- **Known limitations:** browser checks (`tests/ui/*.mjs`) stay manual, as before; card ages use the last update, since
  cards carry no claim time. Cards have no target dates, so the roadmap timeline shows history up to today, not a
  plan ahead.

---
category: Added
changelog: rusty-mux-gui gains a session switcher under the session chip, and its window thumbnails light the blocks that are running.
---
## 2026-11-06 - rusty-mux-gui: session switcher

- **Added:** arc 2 slice S4a (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D60, handoff D108, D109). A click on the session chip opens the switcher: every session on the server with its windows and clients, the current one marked, then Settings and Open config file. Up and Down move, Enter switches, Esc or a click outside closes. A session without blocks to show is greyed out and skipped.
- **Added:** picking a session shows its active window, else its first with blocks, on the same control connection. The window's old blocks are dropped and the new ones opened; no other client's active window moves.
- **Added:** a strip thumbnail's cell takes the accent while its block runs a program (gui-design D46, D60), for every window of the session.
- **Changed:** rusty_term's session-source port gains `Source::switch` and `Session::sessions` (`SessionChoice`). The strip's client count is now the distinct people attached to the session's blocks (handoff D105), not every client on the server. Pixmaps may be see-through, so the switcher's rounded corners show what is under them in both renderers.
- **Known limitations:** after a switch, the session left behind can still count this client for up to 30 s: the server keeps the GUI's dropped block connections for its reconnect grace. One host group only: the protocol does not carry a host name (handoff Q23).

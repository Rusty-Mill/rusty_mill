---
category: Fixed
changelog: rusty_term's TUI paints a short-lived child's last output before it exits.
---
## 2026-10-27 - rusty_term paints the last frame on child exit

- **Fixed:** the TUI no longer drops a short-lived child's output on exit
  (#1162). `rusty_term -- sh -c 'echo LAST'` painted nothing in most runs,
  because SIGCHLD stopped the render loop before the PTY was read. Shutdown now
  reads the PTY to EOF, bounded at 250 ms for a grandchild that holds it open,
  and paints one last frame, releasing a synchronized update the child left
  open. Windows drains ConPTY until it is quiet for 50 ms, within the same
  bound. rt-caps.sh drops its 0.3 s exit sleep.
- **Known limitations:** the Windows drain is compiled and linted in CI but not
  exercised at runtime by a test.

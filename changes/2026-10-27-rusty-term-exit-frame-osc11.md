---
category: Fixed
changelog: rusty_term paints a short-lived child's last output before exiting, and OSC 10/11 and their resets recolor text already on screen.
---
## 2026-10-27 - rusty_term last frame on exit and OSC 11 recolors

- **Fixed:** the TUI no longer drops a short-lived child's output on exit
  (#1162). `rusty_term -- sh -c 'echo LAST'` painted nothing in most runs,
  because SIGCHLD stopped the render loop before the PTY was read. Shutdown now
  reads the PTY to EOF, bounded at 250 ms for a grandchild that holds it open,
  and paints one last frame, releasing a synchronized update the child left
  open. Windows drains ConPTY until it is quiet for 50 ms, within the same
  bound. rt-caps.sh drops its 0.3 s exit sleep.
- **Fixed:** cells drawn in the default colours follow OSC 10/11 and their
  resets, OSC 110/111, as in xterm (#1163). Before, text written while OSC 11
  was set kept that background after OSC 111. A change of the default colour
  now recolors every style still in the old default, across the screen and
  scrollback, role by role (fg for OSC 10, bg for OSC 11). Cells still store
  resolved RGB, so the snapshot wire format is unchanged.
- **Known limitations:** a colour the child set explicitly that equals the
  current default (for example SGR 40 when the background is the theme's
  black) is not told apart from the default and follows the change too. A pen
  saved with DECSC before the change restores the old colour.

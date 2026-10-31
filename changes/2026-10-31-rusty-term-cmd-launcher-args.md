---
category: Security
changelog: rusty_term on Windows quotes launcher arguments for cmd.exe and batch files by cmd's rules, so an argument can no longer run a command (#1279).
---
## 2026-10-31 - rusty_term: launcher arguments can't inject into cmd.exe

- **Security:** on Windows, the arguments after `rusty_term --` were quoted
  by the C runtime's rules even when cmd.exe read them, as `shell = "cmd /c
  echo"`, `-- cmd /c ...` or a batch file. cmd doesn't treat `\"` as an
  escape, so an argument such as `x\" & calc & \"y` closed its quotes and
  ran `calc` (the BatBadBut class, #1279). Both hosts, ConPTY and
  `[console] host = "own"`, now quote these arguments by cmd's rules: a
  plain word stays bare, anything else goes in quotes with `"` doubled, and
  when cmd's `/c` or `/k` command starts with a quote the command is wrapped
  in one more pair, which cmd strips. Other programs keep C runtime quoting.
- **Changed:** for cmd.exe and batch files, an argument with `%`, CR, LF or
  NUL is refused with an error instead of passed on, since cmd expands `%`
  even inside quotes. So is a quoted argument after a shell whose own text
  leaves a quote open, and, before cmd's own `/c`, `/k` or `/r`, an
  argument holding one other than the bare switch: cmd takes the first such
  switch anywhere, even inside quotes, so `cmd` with the argument
  `"x /c calc"` would run calc. Put such text in `shell` itself, which is
  passed as written.

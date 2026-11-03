---
category: Fixed
changelog: On our own console host, rusty_term passes the shell's arguments to it as written, so cmd.exe reads them as it would under ConPTY.
---
## 2026-10-30 - platform: raw command line on Windows; rusty_term keeps cmd quoting

- **Added:** `platform::process::Command::raw_args`, Windows only, like
  std's `CommandExt::raw_arg`. Its text goes on the `CreateProcessW`
  command line verbatim, after the quoted program and before `argv`.
  It is refused with a NUL or a batch-file program, and `Unsupported` on
  Linux (rustils divergence 017). All three Windows spawn paths
  (`WindowsSpawner`, `WindowsPty`, `condrv::spawn_attached`) now build
  their line through one crate-private helper.
- **Fixed:** with `[console] host = "own"`, rusty_term splits only the
  program off `shell` and passes the rest verbatim, so
  `cmd /c echo a"b c"d` reaches cmd as written instead of requoted as
  `/c echo "ab cd"`. A batch-file shell with arguments runs through
  `cmd.exe /c` exactly as `CreateProcessW` runs it.

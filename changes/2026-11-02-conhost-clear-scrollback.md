---
category: Fixed
changelog: On our console host, cmd's cls, PowerShell's Clear-Host and [Console]::Clear() now empty the terminal's scrollback, as under ConPTY.
---
## 2026-11-02 - Shell clears empty the scrollback on our console host

- **Fixed:** on our console host (rusty_conhost), `cls` in cmd and `Clear-Host` or `[Console]::Clear()` in PowerShell cleared the screen but left the terminal's scrollback, which a later shrink brought back onto the screen (#1371). The host now applies conhost's GH#3126 shims: a whole-screen ScrollConsoleScreenBuffer from `cmd.exe`, or a whole-screen FillConsoleOutputCharacterW of spaces from `powershell.exe` or `pwsh.exe`, is sent as `CSI H CSI 2J CSI 3J`, and PowerShell's following fill of the default attribute sends nothing. Other clients and partial fills paint cells as before. A Windows test compares the VT the terminal receives under ConPTY and under our host.
- **Known limitations:** the shell is known by CONNECT's ApplicationName, where conhost reads the client process's image name.

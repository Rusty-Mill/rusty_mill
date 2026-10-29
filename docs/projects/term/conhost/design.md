# rusty_conhost: our own Windows console host (design, draft 4)

- **Issue:** #1222
- **Date:** 2026-10-29
- **Authors:** Claude, for Nano (owner) and Dottius (reviewer)
- **Status:** D1, D5 and D6 approved by Nano; D2 approved by Nano and awaiting Dottius's co-sign on the exact changes in §11.1; D3 awaiting Dottius on the exact signatures in §11.2; D4 decided by Dottius (§8.3). D6 landed as #1227. Draft 4 adds those exact forms. Drafts 1–3 went through two Elephant Goldfish rounds (independent AI reviewers, one aspect each):
  - **Round 1:** one combined pass found 1 P0 and 12 P1.
  - **Round 2:** seven parallel reviewers on draft 2 found no P0. Their P1s are resolved, as Appendix B shows.
- **Evidence,** all in this folder (`docs/projects/term/conhost/`):
  - `recon-2026-10-29.md`: how ConPTY and ConDrv work.
  - `spike-1-2026-10-29.md`: spike code, API census, model gaps.
  - `report-2026-10-29.md`: spike results and estimates.
  - `egm-review-1.md` and `egm-review-2.md`: the two review rounds.
- **Spike code:** 1,318 lines of std-only Rust on Beast, Nano's Windows 11 machine (build 26300.9711), at `C:\dev\conhost-recon\rusty_conhost`, outside the repo.

## Summary and decisions requested

We'll replace conhost, the Windows console host behind ConPTY, with our own host running inside rusty_term. A spike already runs interactive cmd and pwsh/PSReadLine on it with no Microsoft host in the process tree. The work is about 13 PRs. The first milestone, interactive cmd and pwsh in rusty_term behind a config option, lands after PR 5, about 15–22 days in. The full plan is 28–40 days (§10).

| # | Decision | Who | Where | Outcome |
|---|---|---|---|---|
| D1 | Build option C: our own host, in-process. This supersedes the "don't roll our own ConDrv" position in `rusty_term/docs/research/conpty-attach-2026-07.md:206-210` | Nano | §1.2 | Approved (Nano, 2026-10-29) |
| D2 | Approve the dependency, feature, lockfile and admission changes | Nano / Dottius | §11.1 | Nano approved; Dottius to co-sign the exact list |
| D3 | Approve the public API additions | Dottius | §11.2 | Pending: exact signatures and behaviour now in §11.2 |
| D4 | How Windows tests run before review | Dottius | §8.3 | Decided: the existing manual workflow on the exact PR commit before review; merge-queue checks unchanged |
| D5 | Phase B criteria for making our host the default | Nano | §9 | Approved as written in §9.2 |
| D6 | Land the separate quick fix: add `STARTF_USESTDHANDLES` to rusty_term's ConPTY spawn | Nano | §1.1 | Approved and merged as #1227; both ConPTY tests run again |

## Glossary

| Term | Meaning |
|---|---|
| ConDrv | The kernel console driver (`\Device\ConDrv`). Console programs talk to their host through it. |
| Host | The process that answers a console's ConDrv traffic: today conhost.exe, later us. |
| Client | A console program attached to a host, such as cmd, pwsh or ping. |
| ConPTY | Windows' pseudo-console API. It starts a headless conhost.exe and exposes VT over pipes. |
| VT | The escape-code byte stream terminals speak. SGR sets colours, OSC sets titles and palettes, DSR/CPR asks for the cursor position, DA1/DA2 asks the terminal's identity. |
| win32-input-mode | A VT input encoding (`CSI ?9001h` turns it on) in which the terminal sends each key as a full Windows key record (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`). rusty_term already speaks it (`rusty_term/src/backend/windows.rs:604-611`). |
| IOCTL | A device control call to a driver. NTSTATUS is the Windows kernel status code. |
| INPUT_RECORD / CHAR_INFO | The Win32 structs for one input event and for one screen cell. |
| ApiNumber | `layer << 24 \| index`. It identifies a console API inside a ConDrv USER_DEFINED packet. |
| Cooked read | `ReadConsole` with line editing done by the host: echo, backspace, history. |
| Sans-I/O | Logic written as a pure state machine. It takes events and returns effects, and never calls the OS. |
| Phase A / B | Phase A: our host is opt-in. Phase B: our host is the default (§9). |
| S1, S2, S3 | Short spikes still owed (§9.1). |
| Track W (D-15) | platform-windows' convention for an optional second FFI arm through `rusty_win32`, behind the `track-w` feature. |
| Change fragment | The per-PR release-note file under `changes/` that CONTRIBUTING requires. |

## 1. Goal

Run Windows console programs under a host we own, with no conhost.exe, OpenConsole.exe or conpty.dll in the process tree. The target programs are cmd, pwsh with PSReadLine, ordinary .exe tools, and TUI tools such as edit.exe. rusty_term will use it first. The rusty_mux server and rusty_yirp's `sessionmgr-pty` crate will use it after the milestone.

### 1.1 Why

- **Ownership.** Owner direction (Nano, 2026-10-29) is to own the whole stack. The console host is the last Microsoft layer between our terminal and the programs it runs.
- **Defects ConPTY has caused:**
  - **Unknown escape sequences are dropped.** The inbox conhost drops escape sequences it doesn't know, so `?2027h` never reaches rusty_term (#1220).
  - **Startup bytes vary by build.** Inbox conhost and Microsoft's open-source main send different startup bytes (recon, "Key findings").
  - **The host keeps a second screen model we can't change,** so widths and grapheme handling are Microsoft's, not rusty_term_core's.
- **Not a ConPTY defect.** The "ConPTY attach broken on Insider 26200.8737" failure (`rusty_term/src/backend/windows.rs:568-573`) is probably our own bug. rusty_term's spawn never sets `STARTF_USESTDHANDLES`. That is the exact leak platform-windows already fixed (`platform-windows/src/sys/pty.rs:330-356`), and the symptoms in `conpty-attach-2026-07.md:16-20` match it. The fix is decision D6 and is independent of this design.
- **What our own host gives us:**
  - rusty_term_core as the host's screen model, so host and terminal agree by construction;
  - control of Ctrl events, process groups and lifetime;
  - a path to a Job Object and `kill_tree`, which is unsupported today (`platform-windows/src/sys/pty.rs:184-196`).

### 1.2 Non-goals

- **A visible console window.** We host headless only, as ConPTY does.
- **Legacy console UI.** No classic window, property dialogs, IME composition UI, console selection or QuickEdit.
- **Programs started outside our terminals.** They keep Windows' own host.
- **Privilege isolation.** The host runs as the user, like conhost. Untrusted input from clients is in scope (§7).
- **32-bit (WOW64) clients in the first landing.** S2 checks whether ConDrv hands us the same layouts for them. ARM64 native clients use the 64-bit layouts.

### 1.3 Alternatives considered

| Option | Fixes | Cost | Ongoing burden | Main risk |
|---|---|---|---|---|
| A. Status quo (inbox ConPTY) | nothing | 0 days | Microsoft's bugs, which we can't fix | more drift like #1220 |
| B. Bundle conpty.dll and OpenConsole.exe (#1220) | passthrough, newer fixes | "M+S" in `conpty-attach-2026-07.md:190-280` | tracking and shipping Microsoft binaries | the screen model is still outside our control |
| **C. Our own host, in-process (this design)** | everything in §1.1 | 28–40 days (§10) | maintaining code against undocumented ConDrv and ConsoleControl | Microsoft changes or locks down ConDrv |
| D. Our own host as a separate exe | as C, plus OS fault isolation | C plus a wrapper; not costed | as C, plus a binary to ship | as C |

We choose C. D is the fallback if the triggers in §3.2 occur. B stays a valid stop gap until Phase B.

## 2. Background: how hosting works

Steps 1–5 and the Ctrl+C part of step 6 are proven by the spike. Step references below are to the spike files.

1. **Create a server.** Open `\Device\ConDrv\Server` with no root directory. The handle is asynchronous because no `FILE_SYNCHRONOUS_IO_*` option is passed. No privilege is needed. The spike used `NtOpenFile`; the product uses the already-admitted `NtCreateFile` with `FILE_OPEN`, which opens the same object (§11.1).
2. **Open the client handles.** With the server handle as RootDirectory, open:
   - `\Reference` (synchronous, not inheritable)
   - `\Input` and `\Output` (inheritable)

   Duplicate `\Output` for stderr.
3. **Register the input event.** `IOCTL_CONDRV_SET_SERVER_INFORMATION` registers a manual-reset event. It must be set while the input queue is non-empty and reset on every drain. The spike missed the reset once, and edit.exe spun through 598k reads in 15 s.
4. **Spawn the client.** Call `CreateProcessW` with:
   - `EXTENDED_STARTUPINFO_PRESENT`, `STARTF_USESTDHANDLES` and `bInheritHandles = TRUE`;
   - `PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE` (0x2000A), pointing at the Reference handle;
   - `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, limited to in, out and err.
5. **Serve.** Five IOCTLs are used:
   - `SET_SERVER_INFORMATION` (step 3).
   - `READ_IO`. Its input is the previous reply. Its output is the next descriptor `{Identifier, Process, Object, Function, InputSize, OutputSize}`.
   - `COMPLETE_IO`, for a call finished later from any thread.
   - `READ_INPUT` and `WRITE_OUTPUT`, for variable-length payloads.

   Descriptor and connection details:
   - **Function codes:** 1 CONNECT, 2 DISCONNECT, 3 CREATE_OBJECT, 4 CLOSE_OBJECT, 5 RAW_WRITE, 6 RAW_READ, 7 USER_DEFINED, 8 RAW_FLUSH.
   - **CONNECT** carries Process = PID and Object = TID.
   - **Cookies.** The host replies to CONNECT with three opaque "cookies" `{Process, Input, Output}` (`CD_CONNECTION_INFORMATION`). ConDrv echoes them back in later descriptors.
6. **Ctrl events.** `user32!ConsoleControl(ConsoleEndTask = 7, {pid, hwnd = 0, event, flags})` delivers an event to a process attached to our server.
   - It is silently refused for any other process.
   - NTSTATUS is always 0, so delivery can only be checked by its effect. The spike's test client logs the event and exits 0.
   - The values (`CTRL_C_EVENT` = 0, `CTRL_BREAK_EVENT` = 1, `CTRL_CLOSE_EVENT` = 2, `CONSOLE_CTRL_C_FLAG`, `CONSOLE_CTRL_BREAK_FLAG`, `CONSOLE_CTRL_CLOSE_FLAG`) are taken from microsoft/terminal `src/host/conwinuserrefs.h` (MIT) in PR 1.
   - Ctrl+C is proven. Ctrl+Break and CTRL_CLOSE are S1.

### Spike results

All of these are on Beast:

- **Interactive cmd:** echo, dir, title and exit work, over 115 messages using 9 APIs.
- **Ctrl+C:** stops `ping -t`, and cmd survives it.
- **pwsh/PSReadLine** works with rusty_term_core as the screen: editing, arrows, tab completion, predictions, scrolling and wrap. One session made about 1,100 messages, 373 of them GetConsoleScreenBufferInfo.
- **The next unimplemented APIs** are FillConsoleOutput and ReadConsoleOutput, so `Clear-Host` fails.

## 3. Architecture

### 3.1 Placement

The repo has five layers: foundation, platform, libs, apps and tools. A crate may depend only on its own layer or lower ones (ADR 0003).

| Piece | Crate | Layer |
|---|---|---|
| ConDrv mechanism (`condrv`) | `platform-windows` | platform |
| Console engine and runtime | new `crates/libs/ui/rusty_conhost` | libs |
| Consumers | rusty_term (PR 5), `rusty_mux_server` (PR 11), `sessionmgr-pty` (PR 12) | libs, apps |

All `unsafe` and undocumented calls live in platform-windows, which already owns the Windows FFI. `sys::condrv` is `pub(crate)`, and only the safe wrapper `platform_windows::condrv` is exported. Everything platform-specific in rusty_conhost (`runtime`, `pty`) is `cfg(windows)`. The core is plain Rust and is tested on Linux. The spike report suggested putting everything under platform-windows. This design splits it instead, because the engine needs rusty_term_core (libs), and platform may not depend on libs.

### 3.2 Process model: in-process

The engine runs as threads inside the consumer. That means no extra binary and no signal pipe protocol, and a host bug is contained per console (§6.9). What we give up is OS fault isolation.

**Triggers for option D,** a `rusty_conhost.exe` speaking ConPTY's command line, pipe and signal protocol (recon, "ConPTY pipe protocol"):
- a crash that `catch_unwind` cannot contain, seen in practice;
- the §7 limits failing to protect the mux server;
- a need to host consoles for processes we didn't start.

### 3.3 Sans-I/O core

```rust
pub struct Console { /* objects, clients, input queue, waiting reads, screen, modes */ }

impl Console {
    pub fn new(cfg: ConsoleConfig) -> Self { todo!() }
    /// Pure: no OS calls. Every effect is returned for the caller to run.
    pub fn handle(&mut self, ev: Event<'_>, out: &mut OutputQueue) -> Vec<Effect> { todo!() }
}

pub struct ConsoleConfig {
    pub size: WinSize,                               // platform::term::WinSize
    pub key_for: fn(char) -> Option<(u16, u32)>,     // VK and shift state; condrv::key_for on Windows, a US table in tests
}

pub enum Event<'a> {
    Packet(Packet<'a>),   // a READ_IO packet whose variable input the runtime already read (capped, §7)
    Input(&'a [u8]),      // VT bytes from the terminal
    Resize(WinSize),
    OutputDrained,        // the reader took bytes out of the queue
    ChildExited,
    HangUp,
    GraceExpired,         // the runtime's CTRL_CLOSE grace timer fired (§6.8)
}

pub enum Effect {
    Reply(Reply),                     // sent with the next READ_IO
    Complete(Reply),                  // a waiting call finishes now (COMPLETE_IO)
    SetInputAvailable(bool),          // the ConDrv input event
    Ctrl { pid: u32, event: CtrlEvent },
    CloseServer,                      // disconnect the remaining clients
    Eof,                              // the reader gets Ok(0) once the queue drains
}
```

**Types the core owns:**

- **`Packet<'a>`:** the descriptor fields, plus `payload: &'a [u8]` holding the variable input.
- **`Reply`:**
  - `id: Identifier` (a `[u8; 8]` LUID);
  - `status: i32` (NTSTATUS);
  - `information: usize`;
  - `write_back: Vec<u8>`, the fixed API struct returned through `CD_IO_COMPLETE.Write`;
  - `output: Vec<u8>`, the variable output.
- **`CtrlEvent`:** `C | Break | Close`.

**Payload order on the driver thread:**
1. `read_io` delivers the descriptor.
2. The runtime calls `read_input` for up to the capped `InputSize`.
3. The runtime calls `handle(Packet)`.
4. For each `Reply`, the runtime calls `write_output(output)` and then completes the call.

**Lock rule:**
- The runtime holds `Mutex<Console>` only across `handle`.
- `Emit` doesn't exist as an effect. `handle` appends VT to the in-memory `OutputQueue` under the same lock, so output order matches the order calls were handled.
- Every OS call (`Reply`/`Complete`/`SetInputAvailable`/`Ctrl`/`CloseServer`) runs after unlocking. This removes the deadlock where a client's Ctrl handler calls WriteConsole while delivery is still running, and a core test reproduces that sequence.

**Tests and adapters:**
- No driver trait is defined. The core is tested by feeding `Event` sequences to `handle` and asserting on effects and queue contents.
- The runtime calls `platform_windows::condrv` directly and is covered by Windows integration tests.
- A trait arrives only if a second real adapter does (option D).

### 3.4 Threads and data flow per console

```text
 terminal (rusty_term)                                    clients (cmd, pwsh, ...)
  write(VT) --> [caller thread] Event::Input                  |  console API calls
  read()    <-- OutputQueue (soft bound 64 KiB) --+           v
  resize()  --> [caller thread] Event::Resize     |       ConDrv (kernel)
                     |                            |           |  READ_IO / COMPLETE_IO
                     +---> Mutex<Console>::handle(ev, &mut queue) <---- [driver thread]
                     ^             | effects run after unlock
             [exit watcher] Event::ChildExited / GraceExpired
```

- **Driver thread.** Runs the READ_IO loop. Shutdown cancels its pending READ_IO with `CancelIoEx`.
- **Caller threads.** `PtyMaster::write`, `resize`, `hang_up` and `read` (which raises `OutputDrained`) call `handle` on the caller's own thread, under the same lock rule.
- **Exit watcher.** Waits on a duplicate of the direct child's process handle, returned by `condrv::spawn_attached`, then raises `ChildExited`. It also owns the grace timer.

**Backpressure** (soft bound):
- `handle` always accepts the output of the call it is serving.
- While the queue is above 64 KiB, *new* WriteConsole and RAW_WRITE calls are parked on the same wait list as ReadConsole. `OutputDrained` completes them in order.
- Output the host itself originates is exempt, so it can't deadlock: echo, CPR replies, `^C`.

A slow reader therefore stalls clients, as a full ConPTY pipe does. Memory stays bounded at 64 KiB plus one call's output. The mux's contract is "an unread pty blocks the child" (`rusty_mux/crates/server/src/pty/windows.rs:31-33`, `QUEUE = 4` chunks), and this holds that contract.

### 3.5 Modules in `rusty_conhost`

| Module | Job | Spike source |
|---|---|---|
| `msg` | Typed decode/encode of the descriptor, CONNECT info and one struct per supported API. Bounds-checked with `rusty_wire` | dispatch.rs, part (245 lines) |
| `console` | `Console::handle`, the object table, clients and their `ProcessGroupId` | dispatch.rs |
| `input` | INPUT_RECORD queue, VT and win32-input-mode parser, raw passthrough, terminal-reply filter, waiting reads | input.rs (128), vtinput.rs (122) |
| `cooked` | ReadConsole line discipline | readconsole.rs (120) |
| `screen` | Screen over `rusty_term_core::Grid` (scrollback 0) and `AnsiParser`, attribute mapping, read-back | screen.rs (122) |
| `ctrl` | Which pids get which event | ctrl.rs (71) |
| `runtime` (windows) | Threads, the lock rule, panic path, grace timer | main.rs (208) |
| `pty` (windows) | `OwnPty`, `OwnPtyMaster` (§4) | new |

The platform mechanism corresponds to the spike's driver.rs (211 lines).

**Layouts and notice:**
- Struct layouts are transcribed from microsoft/terminal at commit 09beddc6: `dep/Console/condrv.h`, `conmsgl1.h`, `conmsgl2.h`, `conmsgl3.h` and `src/server/ApiMessage.h`.
- Every file with transcribed layouts or ported logic (`msg`, `cooked`, `input`, `ctrl`, `condrv`) carries the MIT notice, and the crate's `THIRD_PARTY_NOTICES.md` records it (§11.1).
- **Possible reuse:** `rusty_lines` has a private VT key decoder (`rusty_lines/src/lib.rs:1598-1660`). It lacks win32-input-mode, so `input` ports the spike's parser instead.

## 4. Interfaces

### 4.1 `platform_windows::condrv` (PR 1, public API)

```rust
pub struct ConsoleServer { /* server, reference, input/output handles, input event */ }
pub struct Attached {
    pub server: ConsoleServer,
    pub child: WindowsChild,        // built inside platform-windows; from_parts stays pub(crate)
    pub wait: OwnedWinHandle,       // a duplicate of the child's process handle, for the exit watcher
}
pub struct IoDescriptor { pub id: [u8; 8], pub process: usize, pub object: usize,
                          pub function: u32, pub input_size: u32, pub output_size: u32 }
pub struct IoComplete<'a> { pub id: [u8; 8], pub status: i32, pub information: usize,
                            pub write_back: &'a [u8] }
pub enum CtrlEvent { C, Break, Close }

/// Ok if ConDrv server creation and user32!ConsoleControl are both available,
/// else ErrorKind::Unsupported.
pub fn probe() -> Result<()> { todo!() }
pub fn spawn_attached(cmd: &Command) -> Result<Attached> { todo!() }
pub fn send_ctrl(pid: u32, event: CtrlEvent) -> Result<()> { todo!() }

impl ConsoleServer {
    pub fn read_io(&self, reply: Option<&IoComplete<'_>>) -> Result<IoDescriptor> { todo!() }
    pub fn complete_io(&self, c: &IoComplete<'_>) -> Result<()> { todo!() }
    pub fn read_input(&self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize> { todo!() }
    pub fn write_output(&self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()> { todo!() }
    pub fn set_input_available(&self, on: bool) -> Result<()> { todo!() }
    pub fn cancel(&self) { todo!() }    // CancelIoEx on the pending READ_IO
    pub fn close(self) { todo!() }      // closes the server handle; ConDrv disconnects the clients
}
```

`Result` is `platform::error::Result`, and `Unsupported` is `ErrorKind::Unsupported`. PR 3 adds `key_for` and PR 9 adds the code-page functions; §11.2.1 gives every exact signature and its behaviour.

**What `spawn_attached` handles for the caller:**
- It resolves the program with `WindowsSpawner::resolve` and builds the command line with winargv, as `WindowsPty::spawn_concrete` does today (`platform-windows/src/pty.rs:60-65`).
- It takes the existing `SPAWN` lock, which becomes `pub(crate)`.
- The server is opened with the already-admitted `NtCreateFile` (§11.1), not a new `NtOpenFile` admission.
- It sizes the proc-thread attribute list for two attributes, with an env, cwd and attribute-list helper factored out of `sys::pty::spawn_attached`.

### 4.2 `rusty_conhost` consumer types (PR 2, public API)

```rust
pub struct OwnPty;
impl OwnPty {
    pub fn spawn_concrete(&self, cmd: &Command, size: WinSize)
        -> Result<(OwnPtyMaster, WindowsChild)> { todo!() }
}
impl platform::pty::Pty for OwnPty { /* boxes spawn_concrete */ }

pub struct OwnPtyMaster { /* Arc<Mutex<Console>>, queue, runtime handles */ }
impl platform::pty::PtyMaster for OwnPtyMaster { /* read, write, resize */ }
impl OwnPtyMaster {
    pub fn hang_up(&self) { todo!() }   // non-blocking, §6.8
}
```

- `OwnPtyMaster` is `Send + Sync`. All of its state sits behind `Arc` and `Mutex`.
- Nothing else is exported. We don't add an enum over ConPTY and our host until a consumer needs one. The mux and yirp each get a small local choice in their own PR.

### 4.3 rusty_term_core additions (PR 4, public API)

1. `AnsiParser::pen_colors(&self) -> CellColors`, so GetConsoleScreenBufferInfo `wAttributes` follows VT SGR, reverse and underline included. `Pen` itself stays private (`mod cell` is private, `core/src/lib.rs:29`).
2. `Grid::cell_colors(&self, col: usize, row: usize) -> Option<CellColors>`. It reads the existing provenance tags in `CellStyle` (`core/src/style.rs:29-42`) and the public `ATTR_FG_DEFAULT`/`ATTR_BG_DEFAULT` bits.
   ```rust
   pub struct CellColors { pub fg: ColorRef, pub bg: ColorRef, pub flags: u32 }
   pub enum ColorRef { Default, Palette(u8), Rgb(u32) }
   ```
3. `AnsiParser::palette(&self) -> &[u32; 256]` returns the live palette, including OSC 4 changes (`core/src/parser.rs:125-128`).
4. `Grid::resize_clip(&mut self, cols: usize, rows: usize)` clips or extends rows without reflow and clamps the cursor. `Grid::resize` always reflows (`core/src/grid.rs:3656-3669`).
5. Fix the stale autowrap doc comment (`core/src/parser.rs:1082`). DECAWM works (`parser.rs:1271-1272`), so `ENABLE_WRAP_AT_EOL_OUTPUT` is emitted as `CSI ?7h` / `CSI ?7l`.

None of these changes the wire format, so RTCP and RTSN (the mux checkpoint and snapshot formats) don't bump.

### 4.4 Selecting the host

- **rusty_term:** a new config key `[console] host = "conpty" | "own"`, default `"conpty"`. It's parsed in PR 2 and documented in PR 5. It goes under `[console]` rather than `[windows]` so it isn't one letter away from the existing `[window]` section (`rusty_term/src/config.rs:736-788`).
- **rusty_mux server:** there's no config system (`rusty_mux/crates/server/src/lib.rs:49-56`). PR 11 adds a `console_host` field to `Config`, which is set by the env var `RUSTY_CONSOLE_HOST`. It also updates the struct-literal sites (`conn.rs:132`, `state.rs:1019, 1040`, `tests.rs:21`).
- **yirp (`sessionmgr-pty`):** PR 12 adds `PtyOptions::console_host`, using the same env var.
- **No silent fallback.** If `own` fails (`Unsupported`, for example), the error is shown.

### 4.5 rusty_term wiring (PR 5)

A second `BackendHandle` implementation, `OwnBackend`, wraps `(OwnPtyMaster, WindowsChild)`. The ConPTY backend stays as it is. Unifying the two is a separate follow-up issue after Phase B.

| BackendHandle method | OwnBackend |
|---|---|
| `read_into`, `output_ready` | `OwnPtyMaster::read`, and whether the queue is non-empty |
| `write`, `resize` | `OwnPtyMaster::write` / `resize` |
| `try_clone` | Clones the `Arc` handles. Only the original reaps the child |
| `exit_token` | Signalled by the exit watcher (`ChildExited`) |
| `reap_exit_status` | `WindowsChild::try_wait` |
| Drop | `hang_up`. After 2 s, kill the direct child, matching today (`rusty_term/src/backend/windows.rs:509-516`) |

`spawn_shell` builds a `platform::process::Command`:
- The `shell` string is split with rusty_term's existing argv splitting, following the `backend/mod.rs:4-8` contract.
- `cwd: None` becomes the current directory.
- The second caller, `web_bridge/mod.rs:344`, uses the same path.

## 5. Behaviour decisions

### 5.1 Buffer and window
We report a buffer the same size as the window, as ConPTY does. Scrollback belongs to the terminal. SetConsoleScreenBufferSize and SetConsoleWindowInfo succeed only at the window size and return STATUS_INVALID_PARAMETER otherwise. This is divergence V1 (§5.10).

### 5.2 Attributes and colours
- **Writes.** Legacy BGR+intensity maps to SGR: 30–37 and 90–97 for foreground, 40–47 and 100–107 for background. `0x07` on `0x00` maps to 39/49. COMMON_LVB_UNDERSCORE maps to 4, and COMMON_LVB_REVERSE_VIDEO to 7.
- **Reads** use `cell_colors`:
  - `Palette(0..=15)` is exact.
  - `Default` maps to 7 for the foreground and 0 for the background.
  - `Palette(16..)` and `Rgb` take the nearest of the 16 entries in `palette()`.
- **The colour table has a single owner: the host parser's palette.** It sees every OSC 4 a client writes. GetConsoleScreenBufferInfoEx reports it. SetConsoleScreenBufferInfoEx is accepted as a no-op, which is divergence V2.

### 5.3 Width and graphemes
rusty_term_core's width rules apply. A wide cell reports COMMON_LVB_LEADING_BYTE and TRAILING_BYTE, taken from WIDE_TRAILER. A cell that can't round-trip one UTF-16 unit reads back as U+FFFD, which is conhost's limit too.

### 5.4 Output modes
Mode names drop the `ENABLE_` prefix. The classes are exclusive, and each row is tested.

| Mode | Byte class | Emitted to the Grid and terminal |
|---|---|---|
| VIRTUAL_TERMINAL_PROCESSING on | all bytes | as written, except LF (see rule 1 below) |
| VIRTUAL_TERMINAL_PROCESSING off, PROCESSED_OUTPUT on | ESC | U+2190 ←, as conhost shows it |
| VIRTUAL_TERMINAL_PROCESSING off, PROCESSED_OUTPUT on | BS, TAB, BEL, CR, LF | the control's meaning |
| PROCESSED_OUTPUT off (VIRTUAL_TERMINAL_PROCESSING ignored) | any C0 control | its CP437 glyph |

Two rules apply on top of the table:
1. When PROCESSED_OUTPUT is on and DISABLE_NEWLINE_AUTO_RETURN is off, LF is emitted as CR LF.
2. A change to WRAP_AT_EOL_OUTPUT emits `CSI ?7h` or `CSI ?7l`.

### 5.5 Input and Ctrl events
At startup the host emits `CSI ?9001h CSI ?1004h`, as inbox conhost does. This turns on win32-input-mode and focus reports, so Ctrl+Break and exact key records arrive from the terminal.

| Input | Mode | Effect |
|---|---|---|
| Ctrl+C (`0x03`, or a win32-input-mode record) | PROCESSED_INPUT on | `Ctrl{C}` to **every attached client**, matching Windows, where processes started with CREATE_NEW_PROCESS_GROUP ignore it. A pending cooked read ends empty. Nothing is queued. |
| the same | PROCESSED_INPUT off | queued as a key record (`uChar` 3) |
| Ctrl+Break (a win32-input-mode record) | any | `Ctrl{Break}` to every attached client. Never queued. |
| text, keys, VT | VIRTUAL_TERMINAL_INPUT on | passed through raw, as character records |
| text, keys, VT | VIRTUAL_TERMINAL_INPUT off | parsed into KEY_EVENT records (`key_for`) |
| a terminal reply (§5.6) | VIRTUAL_TERMINAL_INPUT on | passed through raw |
| a terminal reply (§5.6) | VIRTUAL_TERMINAL_INPUT off | dropped |

- `GenerateConsoleCtrlEvent(event, group)` goes to the clients whose CONNECT `ProcessGroupId` equals `group`, or to every client when `group` is 0.
- Process-group tracking lands with input in PR 3.

### 5.6 Terminal queries
The host's own `AnsiParser` already answers DSR 5, CPR and DA1/DA2 into `take_responses` (`core/src/parser.rs:91-95, 426, 1670-1690`). After each `advance`, the host drains those responses:
- It keeps DSR 5 and CPR, which describe the host's Grid and so are true for the client. They are queued as input when VIRTUAL_TERMINAL_INPUT is on.
- It strips the matching query from the VT it forwards, so the terminal doesn't answer it too.
- It discards every other response.

DA1/DA2, XTVERSION, DECRQM and the OSC 4/10/11 queries go to the terminal, which owns the reported state. Its replies come back on input. Replies are recognised by their grammar:
- `CSI ? … c`
- `CSI > … c`
- `CSI … $ y`
- `OSC … ST`
- `DCS … ST`

A key sequence with the same bytes is ambiguous, as it is in conhost. This covers what edit.exe needs: CPR and DA1 replies, and OSC colour replies.

### 5.7 Resize
- The host calls `Grid::resize_clip`, which doesn't reflow, and emits nothing.
- It queues a WINDOW_BUFFER_SIZE_EVENT for clients that enabled WINDOW_INPUT.
- The terminal keeps its own reflow. Clients that redraw (PSReadLine, TUIs) redraw on the event.
- Clients that don't redraw (cmd) can, until they next print, see ReadConsoleOutput content that differs from the terminal's reflowed view. That is divergence V3, which S3 measures.

### 5.8 Lifetime
**EOF** comes when the direct child exits and the queue drains. That matches today's `spawn_exit_watcher` (`platform-windows/src/sys/pty.rs:624-643`). Grandchildren (`start /b`) don't keep the console alive.

**Shutdown sequence:** on `ChildExited` or `HangUp`:
1. Send `Ctrl{Close}` to the remaining clients and start a 2 s grace timer. The timer runs on the exit watcher, not on the caller. 2 s matches today's Drop drain bound (`sys/pty.rs:691, 727`).
2. On `GraceExpired`, or when the last client disconnects, `CloseServer`, which disconnects the clients.
3. `Eof` once the queue drains.

**`hang_up` contract:** `hang_up` returns at once. Today it blocks only on a full pipe (`platform-windows/src/pty.rs:147-160`). The mux's `GRACE` = 1 s kill (`rusty_mux/crates/server/src/pty/mod.rs:29`) still works on top of it.

**Before PR 8, and if S1 shows CTRL_CLOSE fails:** step 1 is skipped. The server closes at once, and Drop kills the direct child after 2 s (§4.5).

### 5.9 Code pages
- **PR 3:** GetConsoleCP and GetConsoleOutputCP report 65001. That is divergence V4: ConPTY reports the OEM code page, but every consumer here speaks UTF-8. SetConsoleCP(65001) succeeds. Other code pages get STATUS_NOT_SUPPORTED, and the `A` APIs treat their bytes as UTF-8.
- **PR 9, before Phase B:** any installed code page is accepted. Conversion goes through a `codec` function supplied in `ConsoleConfig`, the same way `key_for` is. It is `MultiByteToWideChar` on Windows and a table for 437/850 in tests, so `chcp 850/932` works.

### 5.10 Known divergences from conhost

| # | Behaviour | conhost | We | Why |
|---|---|---|---|---|
| V1 | Buffer larger than window | allowed (e.g. 9001 rows) | window size only | scrollback belongs to the terminal; ConPTY does the same |
| V2 | SetConsoleScreenBufferInfoEx colours | changes the palette | no-op | no target program needs it |
| V3 | Content after resize, for clients that don't redraw | reflowed by conhost | not reflowed | one owner of reflow |
| V4 | Startup code page | OEM (e.g. 437) | 65001 | consumers are UTF-8 |
| V5 | GetConsoleWindow | a hidden pseudo-window | NULL | add a message-only window if a program in the census needs it |
| V6 | Multiple screen buffers (CREATE_OBJECT type 3, SetConsoleActiveScreenBuffer) | supported | STATUS_NOT_IMPLEMENTED | no target program uses them (edit.exe uses `?1049h`); build when the unknown-API log shows a real caller |

The parity suite (§8) treats each of these as an expected difference.

### 5.11 Required API set
`§10` is the single source for what goes into each PR. ApiNumbers for each API are in Appendix A. In short:
- PR 2: connect/objects, the output-only APIs and stubs.
- PR 3: input, cooked read and Ctrl.
- PR 4: cursor, attributes, SBI and the cursor-info calls.
- PR 6: Fill and Scroll.
- PR 7: the Read/WriteConsoleOutput family and the window/buffer calls.
- Anything else gets STATUS_NOT_IMPLEMENTED, logged once per (process, ApiNumber) through the `log` facility the consumer already uses.

## 6. Robustness

### 6.1–6.7 Limits (security and availability)

| Limit | Value | Where it comes from | On breach |
|---|---|---|---|
| Variable input read per packet | 64 KiB. Read APIs may go to their declared capacity, up to 1 MiB | the spike's largest real request was 16,404 bytes (cmd ReadConsole) | STATUS_INVALID_PARAMETER |
| Output queue | soft 64 KiB (§3.4) | the mux's 4-chunk contract | park the caller |
| Input queue | 4,096 records | about 1 s of fast typing or paste at 4K keys/s | oldest dropped, logged once |
| Waiting reads per console | 64 | cmd uses 1 and PSReadLine uses 1 | STATUS_NO_MEMORY |

Further robustness points:
- **Decoding:** client sizes, offsets and ApiNumbers are never trusted before decoding (`msg`, fuzzed in PR 13).
- **Handle inheritance:** the handle list is limited to in, out and err (§2 step 4).
- **Logs:** capture fixtures are sanitised (§8.2), so logs and fixtures never carry CONNECT's app path, title, current directory or environment.

### 6.9 Panics
- **Containment.** `handle` runs inside `catch_unwind(AssertUnwindSafe(|| console.handle(ev, &mut q)))`.
- **On a panic,** the console is marked dead, and every pending call completes with STATUS_UNSUCCESSFUL. After that, `CloseServer` and `Eof` run.
- **Before each `handle`,** the runtime checks the dead flag. New packets on a dead console are answered STATUS_UNSUCCESSFUL straight away.
- **Poisoned locks** are taken with `unwrap_or_else(PoisonError::into_inner)` (`platform-windows/src/pty.rs:67`).
- **Library code** has no `unwrap`.

## 7. (reserved)

## 8. Testing

### 8.1 Test layers

| Layer | Runs on | Oracle | First PR |
|---|---|---|---|
| Core: `Event` sequences into `handle` | Linux PR CI | golden effects and golden VT per API; the §5.4–§5.6 tables; lock-rule, backpressure, panic and resize scenarios | 2 |
| condrv integration | Windows | the probe's own observed result (exit code, logged event) | 1 |
| Parity: each probe run under ConPTY and under our host, fields compared | Windows | inbox conhost's answer, minus the §5.10 divergences | 2, and each API PR adds probes |
| Fuzz: `msg` decode and packet sequences | nightly | no panic, every effect within limits | 13 |
| Manual gate | Beast plus one older build | see below | each milestone |

- **Probes** are `examples/probe.rs` in rusty_conhost: one binary with a subcommand per API. Tests spawn it from the target directory. No extra workspace member is needed.
- **Platform tests** reuse `platform-windows/tests/pty.rs`'s bounded-read helpers (`sys::pty::wait_readable`, `sys/pty.rs:524-567`).

**The manual gate covers:**
- interactive cmd;
- pwsh/PSReadLine: Clear-Host, completion, history;
- edit.exe;
- `ping -t` with Ctrl+C;
- a resize while running;
- `cmd /c start /b`;
- `chcp 850` (from PR 9);
- rt-caps.sh (`rusty_term/extra/rt-caps.sh`, the repo's terminal capability tour) inside WSL.

The older build is the oldest Windows 10 or 11 machine available; S2 names it.

### 8.2 Fixtures
- Text files live in `rusty_conhost/tests/fixtures/`. Each is a list of steps (`packet`, `input`, `expect`) with no timing.
- A sanitiser scrubs CONNECT fields, paths, environment and user history.
- Each file is capped at 64 KiB.
- PR 2 ships `cmd /c` plus synthetic fixtures. Interactive fixtures are added with the PRs that make them work, re-recorded with the new host's own logger.

### 8.3 Windows tests before review
PR CI runs Linux plus Windows clippy only (`.github/workflows/ci.yml:41-43`). Windows tests first run in the merge queue, which agents can't add PRs to.

**Decided (D4, Dottius, 2026-10-29):** before a PR is put up for review, the existing manual Windows CI workflow (`workflow_dispatch` on `ci.yml`, a full sweep including `windows-latest`, `.github/scripts/ci_plan.py:111-151`) runs on the PR branch at the exact head commit. The PR body links that run and names its commit. A push after the run means a new run before re-review. The normal merge-queue checks stay required. This is not a CI change. A Beast test log may be attached as extra evidence, never instead of the run.

## 9. Rollout

### 9.1 Spikes still owed

| Spike | Question | Where | Blocks | Estimate |
|---|---|---|---|---|
| S1 | Do CTRL_CLOSE and Ctrl+Break work through ConsoleControl? What do clients do when the server closes? | Beast | PR 8 | 0.5 days |
| S2 | ConDrv and ConsoleControl on a GitHub `windows-latest` runner (build number), WOW64 layouts, and the oldest machine available | runner via `workflow_dispatch`, Beast | PR 1 acceptance | 0.5 days |
| S3 | How visible is V3 (§5.7) with cmd, PSReadLine and edit.exe? | Beast | Phase B | 0.5 days |

### 9.2 Phases
- **Phase A:** `own` is opt-in from PR 5.
- **Phase B:** `own` becomes the default once all of these hold:
  - the parity suite passes;
  - the manual gate is green on two builds;
  - end-to-end throughput is within 10% of `conpty`. That uses a new Windows bench in PR 14 that pipes the `bench/gen_workloads.py` corpus through each master. The existing gate measures only the parser and grid, under cachegrind on Linux.
- **Rollback:** set `host = "conpty"`. The ConPTY path stays for builds where `probe()` returns `Unsupported`, and we review its future one year after Phase B.
- **#1220** stays useful until Phase B, then closes as superseded.

## 10. PR sequence and estimate

Each PR includes one change fragment and is reviewed by Dottius. Line counts include tests. Days are focused engineer-days.

| PR | Scope | Lines | Days |
|---|---|---|---|
| 1 | `condrv`. Covers: rootless Server open (the admitted `NtCreateFile`, its admission comment extended), relative opens through `sys::nt::open_relative`, the 5 IOCTLs through the admitted `DeviceIoControl`, `spawn_attached`, `send_ctrl` via `GetProcAddress`, `probe()`. Integration tests: open the server, spawn the probe, CONNECT/DISCONNECT round trip, Ctrl+C observed by the probe's exit code, `Unsupported` path. Acceptance: Windows run log | 450 | 2–3 |
| 2 | `rusty_conhost` core plus runtime, output only. Covers: `msg`, `console`, output APIs and stubs (Appendix A), bounded queue, panic path, `OwnPty`/`OwnPtyMaster`, the rusty_term key parsed but undocumented, the probe example, `cmd /c` fixtures. Interim: input is discarded with one log line, resize only updates the size, Drop closes the server and kills the child after 2 s | 900 | 5–7 |
| 3 | Input. Covers: VT/win32-input-mode parser, raw passthrough, reply filter, waiting reads, input event, cooked ReadConsole, Ctrl+C/Break, process groups, GenerateConsoleCtrlEvent, 65001-only code pages. Interactive cmd works | 700 | 3–4 |
| 4 | Screen over Grid. Covers: rusty_term_core additions (§4.3, flagged), cursor/attribute/cursor-info APIs, SBI from the Grid, output-mode table, query routing (§5.6), resize (§5.7). pwsh/PSReadLine and edit.exe start work | 700 | 3–4 |
| 5 | rusty_term `OwnBackend` (§4.5), key documented. **Milestone** | 300 | 1–2 |
| 6 | FillConsoleOutput, ScrollConsoleScreenBuffer. Clear-Host works | 450 | 2 |
| 7 | Read/WriteConsoleOutput family, SBIEx, window/buffer size calls | 550 | 2–3 |
| 8 | Lifetime: CTRL_CLOSE, grace timer, `hang_up` (§5.8). Needs S1 | 300 | 1–2 |
| 9 | Code pages via `codec` (§5.9) | 250 | 1–2 |
| 10 | (reserved for issues found at the milestone) | — | 2 |
| 11 | rusty_mux server: local host choice, `RUSTY_CONSOLE_HOST`, Config field and sites | 250 | 1–2 |
| 12 | yirp `sessionmgr-pty`: `PtyOptions::console_host`, Send/Sync justification (`sessionmgr-pty/src/lib.rs:99-113, 302-305`) | 150 | 1 |
| 13 | Fuzz crate `rusty_conhost/fuzz` plus a nightly job modelled on `rusty-tls-fuzz` (`ci.yml:1673-1714`) | 200 | 1–2 |
| 14 | Windows end-to-end bench and Phase B gate | 250 | 1–2 |
| | **Total** | **~5.5k** | **26–38**, plus S1–S3 (1.5) and per-PR Windows runs (~0.5 per PR for the manual Windows run) **= 28–40** |

- **Milestone:** after PR 5, about 14–20 days plus overheads, so **15–22 days in**.
- **Compared with the spike report:** its 12–20 days covered only "cmd, pwsh and edit.exe usable". This plan adds lifetime, code pages, the mux and yirp, fuzzing, the bench and test infrastructure.
- **Left out:** the rusty_term ConPTY unification and multiple screen buffers (V6) are deferred.

## 11. Approvals

Every change below is proposed in exact form so it can be co-signed as written (D2) or approved as written (D3). Anything not listed here needs its own approval on the PR that introduces it.

### 11.1 D2: dependency, feature, lockfile, admission and CI changes

No third-party crate is added or upgraded anywhere in the plan. The only new external-to-the-workspace dependency is `libfuzzer-sys = "0.4"` in an excluded fuzz crate (PR 13), and three existing fuzz crates already use it at that version. windows-sys features don't appear in `Cargo.lock`, so feature changes don't change any lockfile.

**PR 1: platform-windows** (`crates/platform/rustils/crates/platform-windows`)

| Change | Exact form |
|---|---|
| windows-sys feature | Add `"Win32_System_LibraryLoader"` to platform-windows' own `[target.'cfg(windows)'.dependencies] windows-sys` feature list, next to `Win32_Security_Authorization`. The workspace list in the root `Cargo.toml` is not touched. It provides `LoadLibraryExW` and `GetProcAddress`. |
| Win32 admissions (`src/ffi/win32_surface.rs`) | `LoadLibraryExW`, `GetProcAddress`, `LOAD_LIBRARY_SEARCH_SYSTEM32` (LibraryLoader); `CreateEventW`, `ResetEvent`, `SetEvent`, `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (Threading); `OVERLAPPED`, `GetOverlappedResult`, `CancelIoEx` (IO). `DeviceIoControl`, `UpdateProcThreadAttribute`, `DuplicateHandle`, `WaitForMultipleObjects` and `VkKeyScanW` are already admitted. Each new entry gets a one-line rationale comment, matching the file's style. |
| NT admission (`src/ffi/nt_surface.rs`) | **No new ntdll export.** The rootless open of `\Device\ConDrv\Server` uses the already-admitted `NtCreateFile` with `RootDirectory = NULL`, `FILE_OPEN` and `CreateOptions = 0` (an asynchronous handle). The relative opens of `\Reference`, `\Input` and `\Output` use the existing `sys::nt::open_relative`. That is a new *use* of an admitted export, so the admission comment gains a paragraph with the rationale: Win32 can't open a device object relative to a handle, and ConDrv's sub-objects only exist relative to the server handle. Draft 3 proposed admitting `NtOpenFile`; that's withdrawn. |
| Undocumented export | `user32!ConsoleControl`, resolved at runtime with `LoadLibraryExW(L"user32.dll", NULL, LOAD_LIBRARY_SEARCH_SYSTEM32)` plus `GetProcAddress`, cached in a `OnceLock`. It's never linked statically, so a build where it's missing degrades to `ErrorKind::Unsupported` rather than failing to load. The signature, `ConsoleEndTask = 7` and its struct are transcribed from microsoft/terminal (MIT), with the notice kept. |
| Local constants | `PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE = 0x0002_000A` and the five `IOCTL_CONDRV_*` codes, each defined in `sys::condrv` with a source comment, because windows-sys doesn't ship them. |
| MSRV | `condrv` builds at platform-windows' MSRV 1.75. `msrv-compile` already covers it. |
| Track W | **None.** There is no rusty_win32 surface for ConDrv, and `rusty_win32` needs MSRV 1.88. |
| Visibility | `sys::condrv` is `pub(crate) mod condrv` inside the public `sys` module. The `SPAWN` lock in `pty.rs` becomes `pub(crate)`. |
| Lockfile | None. platform-windows gains no dependency. |

**PR 2: new crate `rusty_conhost`** (`crates/libs/ui/rusty_conhost`)

| Change | Exact form |
|---|---|
| Manifest | `name = "rusty_conhost"`, `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.88"` (the same as rusty_term_core), `license = "MIT"`, `publish = false`, `[package.metadata.rusty_mill] layer = "libs"`, `[lints] workspace = true`. |
| Dependencies | `platform.workspace = true`, `rusty_term_core.workspace = true` (default features, which are none: no `gui`, no `l13`), `rusty_wire.workspace = true`; `[target.'cfg(windows)'.dependencies] platform-windows.workspace = true`. Dev-dependencies: none beyond the crate's own example. |
| Root `Cargo.toml` | Add the path to `members`; add `rusty_conhost = { path = "crates/libs/ui/rusty_conhost" }` to `[workspace.dependencies]`. |
| `Cargo.lock` | One new `[[package]]` entry for `rusty_conhost` listing those four dependencies. No other entry changes, which `lockfile_diff.py` checks. |
| Layers | libs → libs (rusty_term_core), libs → platform, libs → foundation (rusty_wire). All allowed by ADR 0003, checked by `check_workspace_layers.py`. |
| Generated | `docs/WORKSPACE-MAP.md` regenerated with `generate_workspace_map.py`. |
| Licence | `crates/libs/ui/rusty_conhost/THIRD_PARTY_NOTICES.md` carries the microsoft/terminal MIT notice, the same pattern as `rusty_bullet/THIRD_PARTY_NOTICES.md`. The same notice goes at the top of every file with transcribed layouts or ported logic, including `sys::condrv` in PR 1. |

**PR 5: rusty_term.** Add `rusty_conhost.workspace = true` and `platform.workspace = true` under `[target.'cfg(windows)'.dependencies]`. `platform-windows` is already reachable through rusty_conhost, and rusty_term names `WindowsChild` through it. `Cargo.lock`: two names are added to rusty_term's dependency list.

**PR 9: platform-windows.** Add `"Win32_Globalization"` to platform-windows' local feature list, and admit `MultiByteToWideChar`, `WideCharToMultiByte` and `IsValidCodePage`. No lockfile change.

**PR 11: rusty_mux_server.** Add `rusty_conhost.workspace = true` under `[target.'cfg(windows)'.dependencies]`. This is a `rusty_term_core` edge without `l13`, so the mux rule is kept. `Cargo.lock`: one name is added.

**PR 12: yirp `sessionmgr-pty`.** The same edge, `cfg(windows)` only. The crate already takes `platform` through `workspace = true` (`sessionmgr-pty/Cargo.toml:21-24`). `Cargo.lock`: one name is added.

**PR 13: fuzz crate and CI job**

| Change | Exact form |
|---|---|
| Crate | `crates/libs/ui/rusty_conhost/fuzz`, added to the root `exclude` list beside `crates/libs/ui/rusty_term/fuzz`. It has its own `Cargo.lock`, with `libfuzzer-sys = "0.4"` and `rusty_conhost` by path, and two targets: `msg_decode` and `packet_sequence`. |
| CI | A new `rusty-conhost-fuzz` job in `.github/workflows/ci.yml`, modelled on `rusty-tls-fuzz` (`ci.yml:1673-1714`). It uses the same pinned `cargo-fuzz@0.13.2` and runs 30 s per target, on `schedule` and `workflow_dispatch` only, never on PRs or in the merge queue. It is not a required check. This row is a CI change, and it's the only one in the plan. |

**PR 14** adds a Windows bench target inside rusty_conhost, with no new dependencies.

### 11.2 D3: public API, exact signatures and behaviour

Each item is additive. No existing signature changes. `Result` is `platform::error::Result`, and error kinds are `platform::error::ErrorKind`.

#### 11.2.1 `platform_windows::condrv` (PR 1; PR 3 and PR 9 add the last two blocks), all `#[cfg(windows)]`

```rust
pub struct ConsoleServer { /* private */ }          // Send + Sync
pub struct Attached {
    pub server: ConsoleServer,
    pub child: WindowsChild,
    pub wait: OwnedWinHandle,                       // SYNCHRONIZE duplicate of the child's process handle
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoDescriptor {
    pub id: [u8; 8],          // opaque; echo it back unchanged
    pub process: usize,       // the driver's per-client handle from CONNECT
    pub object: usize,        // the driver's per-object handle from CREATE_OBJECT
    pub function: u32,        // 1..=8, Appendix A
    pub input_size: u32,      // bytes readable with read_input
    pub output_size: u32,     // bytes writable with write_output / write_back
}
#[derive(Clone, Copy, Debug)]
pub struct IoComplete<'a> { pub id: [u8; 8], pub status: i32, pub information: usize, pub write_back: &'a [u8] }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlEvent { C, Break, Close }

pub fn probe() -> Result<()>;
pub fn spawn_attached(cmd: &Command) -> Result<Attached>;
pub fn send_ctrl(pid: u32, event: CtrlEvent) -> Result<()>;

impl ConsoleServer {
    pub fn read_io(&self, reply: Option<&IoComplete<'_>>) -> Result<IoDescriptor>;
    pub fn complete_io(&self, c: &IoComplete<'_>) -> Result<()>;
    pub fn read_input(&self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize>;
    pub fn write_output(&self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()>;
    pub fn set_input_available(&self, on: bool) -> Result<()>;
    pub fn cancel(&self);
    pub fn close(self);
}

// PR 3
pub fn key_for(ch: char) -> Option<(u16, u32)>;
// PR 9
pub fn is_valid_code_page(cp: u32) -> bool;
pub fn decode(cp: u32, bytes: &[u8]) -> Result<String>;
pub fn encode(cp: u32, text: &str) -> Result<Vec<u8>>;
```

| Item | Behaviour |
|---|---|
| `probe` | Opens and closes a server handle, then resolves `ConsoleControl`. `Ok(())` when both work. `Unsupported` when the device or the export is missing. Any other OS error is mapped by `sys::errmap`. It has no other side effects and can be called any number of times. |
| `spawn_attached` | Resolves and quotes the program exactly as `WindowsPty::spawn_concrete` does, under the same `SPAWN` lock. It opens the server and its `\Reference`, `\Input` and `\Output`. The child is created with `PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE` plus a `HANDLE_LIST` of only the three std handles, `STARTF_USESTDHANDLES`, and `bInheritHandles = TRUE` (the list limits what is inherited). The call returns once `CreateProcessW` succeeds, so the child's CONNECT is the first packet `read_io` returns. `GroupSpec::JoinGroup` gives `InvalidInput`, matching `WindowsPty`. On any failure every handle is closed and no child is left running. Draft 3's `size` parameter is dropped: the size belongs to the engine, not the driver. |
| `send_ctrl` | Calls `ConsoleControl(ConsoleEndTask, {pid, hwnd: 0, event, flags})`, where `event` is CTRL_C, CTRL_BREAK or CTRL_CLOSE. `Ok(())` means the call was made, not that the signal arrived: the driver reports success even for a pid that isn't attached to us, and Windows then drops it. `Unsupported` when the export is missing. It must not be called while holding the engine lock (§3.3). |
| `read_io` | Blocks until the next packet. With `Some(reply)`, that packet is completed and the next one read in a single IOCTL. Only one call can be in flight per server; a second concurrent call returns `WouldBlock`. `Interrupted` after `cancel`. `BrokenPipe` once the server is closed. |
| `complete_io` | Completes one packet without reading the next. It's used for replies that were held back, such as a `ReadConsole` waiting for input. `write_back` longer than the packet's `output_size` gives `InvalidInput`, and nothing is sent. |
| `read_input` / `write_output` | Copy the packet's input payload from `offset`, or write the reply payload at `offset`. They return `InvalidInput` past `input_size` or `output_size`, and `NotFound` if the packet is already complete. `read_input` returns the byte count, which is short only at the end of the payload. |
| `set_input_available` | Sets or resets the manual-reset input-available event that clients wait on. It is idempotent. The engine resets it when the input queue drains (§3.3). |
| `cancel` | `CancelIoEx` on the pending `read_io`. It's a no-op when none is pending, and callable from any thread. |
| `close` / `Drop` | Closes the server handle. The driver disconnects every client, whose pending console calls fail. Nothing blocks. |
| `key_for` | `VkKeyScanW`: `Some((vk, state))`, where `state` holds `SHIFT_PRESSED`, `LEFT_CTRL_PRESSED` and `LEFT_ALT_PRESSED` as `dwControlKeyState` bits. `None` when the active layout has no key for `ch`. |
| `is_valid_code_page` | `IsValidCodePage`. |
| `decode` / `encode` | `MultiByteToWideChar` / `WideCharToMultiByte` without `MB_ERR_INVALID_CHARS`, so invalid bytes become U+FFFD or `?`, as conhost does. An invalid or uninstalled `cp` gives `InvalidInput`. The engine buffers a trailing partial multibyte sequence itself, so these functions see whole sequences only. |

#### 11.2.2 `rusty_conhost` (PR 2), `#[cfg(windows)]`

```rust
pub struct OwnPty;
impl OwnPty {
    pub fn spawn_concrete(&self, cmd: &Command, size: WinSize) -> Result<(OwnPtyMaster, WindowsChild)>;
}
impl platform::pty::Pty for OwnPty { /* boxes spawn_concrete */ }

pub struct OwnPtyMaster { /* private */ }            // Send + Sync
impl platform::pty::PtyMaster for OwnPtyMaster { /* read, write, resize */ }
impl OwnPtyMaster { pub fn hang_up(&self); }
```

| Item | Behaviour |
|---|---|
| `spawn_concrete` | Fails with `Unsupported` when `condrv::probe` fails. There's no silent ConPTY fallback (§4.4). `JoinGroup` gives `InvalidInput`. Otherwise it starts the IO thread and the exit watcher (§3.4) and returns. `size` is the initial window and buffer size; 0 columns or rows gives `InvalidInput`. |
| `read` | Blocks until output is queued, then returns up to `buf.len()` bytes. It returns `Ok(0)` once the direct child has exited and the queue is drained, which is `PtyMaster::read`'s contract and what `WindowsPtyMaster` does. Concurrent readers are serialized. |
| `write` | Queues VT input bytes from the terminal and returns `buf.len()`. It never blocks on the client. It returns `BrokenPipe` after `hang_up` or once the server is closed. |
| `resize` | Applies `Grid::resize_clip`, updates the size that `GetConsoleScreenBufferInfo` reports, and queues a `WINDOW_BUFFER_SIZE_EVENT` when the input mode asks for one. 0 columns or rows gives `InvalidInput`. `BrokenPipe` once closed. |
| `hang_up` | Doesn't block. It sends CTRL_CLOSE to every attached client and returns. The exit watcher closes the server after a 2 s grace (§5.8). It is idempotent. |
| `Drop` | Calls `hang_up`. Its handles are released without blocking the dropping thread. |

#### 11.2.3 rusty_term_core (PR 4)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorRef { Default, Palette(u8), Rgb(u32) }   // Rgb is 0x00RRGGBB
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellColors { pub fg: ColorRef, pub bg: ColorRef, pub flags: u32 }  // flags: ATTR_* bits

impl AnsiParser {
    pub fn pen_colors(&self) -> CellColors;
    pub fn palette(&self) -> &[u32; 256];
}
impl Grid {
    pub fn cell_colors(&self, col: usize, row: usize) -> Option<CellColors>;
    pub fn resize_clip(&mut self, cols: usize, rows: usize);
}
```

| Item | Behaviour |
|---|---|
| `ColorRef` | `Default` means SGR 39 or 49, or a reset. `Palette(n)` means 30–37, 90–97, 40–47, 100–107 or `38;5;n`/`48;5;n`. `Rgb` means `38;2`/`48;2`. Values are as set, before any theme or reverse is applied. |
| `CellColors::flags` | The existing public `ATTR_*` bits. The host reads `ATTR_REVERSE` and `ATTR_UNDERLINE` for `COMMON_LVB_REVERSE_VIDEO` and `COMMON_LVB_UNDERSCORE`. This field is new since draft 3, because `wAttributes` and `ReadConsoleOutput` need those two bits. |
| `pen_colors` | The current SGR pen, the one the next printed character takes. `Pen` itself stays private. |
| `palette` | The live 256-entry palette as `0x00RRGGBB`, including OSC 4 and OSC 104 changes. |
| `cell_colors` | The colours and flags of the cell at (`col`, `row`) in the visible screen, with row 0 at the top. A wide character's continuation cell reports its lead cell. `None` when out of range. |
| `resize_clip` | Resizes without reflow. When columns shrink, rows are truncated; a wide character cut at the edge becomes a blank. When columns grow, rows are padded with blank default cells. When rows shrink, blank rows below the cursor are removed first, then rows from the top go into scrollback. When rows grow, scrollback is not pulled back. The cursor is clamped and the scroll region reset. 0 is treated as 1. `Grid::resize` is unchanged. |

None of these changes the wire format, so RTCP and RTSN (the mux checkpoint and snapshot formats) don't bump.

#### 11.2.4 Configuration surface

| Item | Behaviour |
|---|---|
| rusty_term `[console] host = "conpty" \| "own"` (parsed in PR 2, documented in PR 5) | Default `"conpty"`. Any other value is a config error naming the key, like other enum keys. It's ignored on non-Windows builds, with one debug log line. |
| Env var `RUSTY_CONSOLE_HOST=conpty\|own` (PRs 11, 12) | Read once at mux server start and at yirp `PtyOptions` construction. If it's unset, the default is `conpty`. Any other value is a startup error. |

## 12. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Microsoft changes or locks down ConDrv or ConsoleControl | Medium | Mechanism isolated in `condrv`; `probe()` returns `Unsupported`; Windows tests; `conpty` stays selectable; #1220 as a fallback |
| A program needs an API we don't answer | High while Phase A runs | Unknown-API log, parity suite, default stays `conpty` until Phase B |
| Two screen models cost throughput | Low to medium | Same shape as ConPTY; the Phase B bench gate |
| A client degrades the mux server | Low | The §6 limits. The busy-packet log is diagnostic only; option D if a limit fails |
| Ctrl delivery deadlocks against the driver | Low | The §3.3 lock rule and its test |
| CTRL_CLOSE can't be delivered | Low to medium | S1; the fallback closes the server and kills the child (§5.8) |
| Licence | Low | MIT notice on transcribed and ported files, plus the crate's `THIRD_PARTY_NOTICES.md` (§11.1) |

## 13. Open questions for the owner

1. ~~Phase B criteria (D5)~~: approved as written in §9.2.
2. After Phase B, do we unify rusty_term's ConPTY backend onto platform-windows, or retire it?

---

## Appendix A. API set by PR

Function codes: 1 CONNECT, 2 DISCONNECT, 3 CREATE_OBJECT, 4 CLOSE_OBJECT, 5 RAW_WRITE, 6 RAW_READ, 7 USER_DEFINED, 8 RAW_FLUSH.

| PR | API | ApiNumber |
|---|---|---|
| 2 | CONNECT, DISCONNECT, CREATE_OBJECT (types 1, 2, 4), CLOSE_OBJECT (may arrive after DISCONNECT with Process 0), RAW_WRITE, RAW_FLUSH | function codes |
| 2 | GetConsoleCP | 0x01000000 |
| 2 | GetConsoleMode / SetConsoleMode | 0x01000001 / 0x01000002 |
| 2 | WriteConsole | 0x01000006 |
| 2 | GetConsoleLangId | 0x01000008 |
| 2 | SetConsoleCP | 0x02000004 |
| 2 | GetConsoleTitle / SetConsoleTitle | 0x02000014 / 0x02000015 |
| 3 | GetConsoleInput (also Peek) | 0x01000004 |
| 3 | ReadConsole, RAW_READ | 0x01000005, function code 6 |
| 3 | GenerateConsoleCtrlEvent | 0x02000001 |
| 3 | GetNumberOfConsoleInputEvents, FlushConsoleInputBuffer | from `conmsgl1.h`/`conmsgl2.h`, pinned in PR 3 |
| 4 | GetConsoleCursorInfo / SetConsoleCursorInfo | 0x02000005 / 0x02000006 |
| 4 | GetConsoleScreenBufferInfo | 0x02000007 |
| 4 | SetConsoleCursorPosition | 0x0200000A |
| 4 | SetConsoleTextAttribute | 0x0200000D |
| 4 | GetCurrentConsoleFont | 0x03000004 |
| 4 | GetConsoleWindow (returns NULL, V5) | 0x0300001F |
| 6 | FillConsoleOutput | 0x02000000 |
| 6 | ScrollConsoleScreenBuffer | from `conmsgl2.h`, pinned in PR 6 |
| 7 | ReadConsoleOutput | 0x02000013 |
| 7 | WriteConsoleOutput(+String), ReadConsoleOutputString, Get/SetConsoleScreenBufferInfoEx, GetLargestConsoleWindowSize, SetConsoleScreenBufferSize, SetConsoleWindowInfo | from `conmsgl2.h`, pinned in PR 7 |
| 11 | GetConsoleProcessList | 0x0300002A |
| — | CREATE_OBJECT type 3, SetConsoleActiveScreenBuffer | deferred (V6) |

## Appendix B. How the review findings were resolved

The round-2 consensus items:

| Finding | Reviewers | Resolution |
|---|---|---|
| Day totals inconsistent | all | One derivation (§10); §1.3 cites it |
| S1 gated the wrong PR | 5 | S1 now gates PR 8, the lifetime PR (§9.1) |
| PR 2 needed a crate from PR 3 | 4 | The rusty_term migration is dropped. rusty_term gets a second backend in PR 5 (§4.5) |
| Screen-buffer PR number wrong | 4 | V6, deferred |
| The port adapter can't live in platform; Win32 calls in the core | design, comprehension, codebase | No driver trait; `key_for` and `codec` are injected functions; `runtime` and `pty` are `cfg(windows)` (§3.1, §3.3) |
| Input event outside the effect model | readiness, comprehension | `Effect::SetInputAvailable` (§3.3) |
| Payload fetch order and output write | readiness, design | §3.3 "Payload order" |
| Emit ordering race | readiness | Append under the lock (§3.3) |
| Oversized write parks forever | readiness, design | Soft bound (§3.4) |
| `Pen` not public; `resize` always reflows; palette in parser | codebase, design | §4.3 items 1–4 |
| The ignored ConPTY test is our own bug | codebase | §1.1 and D6 |
| Migration not behaviour-neutral | codebase | Migration dropped (§4.5) |
| Mux and yirp have no config | codebase | §4.4 |
| Feature, admission and fuzz-job approvals missing | codebase | §11.1, in exact form since draft 4 |
| `hang_up` timing | design, codebase | §5.8: non-blocking, grace on the watcher |
| Parser already answers queries | design, codebase | §5.6 |
| win32-input-mode never enabled | design, comprehension | §5.5 startup preamble |
| Over-scoped first landing | zoom-out | Enum, multiple buffers, mux budgets, full code pages and fuzz deferred; milestone after PR 5 |
| Oracle probes arrive last | design, readiness | Probe example from PR 2, grown with each PR (§8.1) |
| No decision summary; divergences scattered; no rollback; no security section | structural, comprehension | Summary table; §5.10; §9.2; §6 |
| Undefined terms, full paths, Rust sigs, hex case, naming | style, comprehension | Glossary, full paths, compiling sigs, uppercase hex, consistent consumer names |

Round 1's disposition table is in `egm-review-1.md`, in this folder. Every round-1 P0 and P1 is either covered by the rows above or by the section it named.

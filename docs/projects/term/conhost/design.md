# rusty_conhost: our own Windows console host (design, draft 6)

- **Issue:** #1222
- **Date:** 2026-10-29
- **Authors:** Claude, for Nano (owner) and Dottius (reviewer)
- **Status:** draft 6 answers Dottius's re-review of draft 5 on #1229 (round 4, Appendix B); draft 5 answered round 3. D1 to D6 are all decided. Dottius accepted D3 at the design-contract level on draft 6 (`a3290ce6`); implementation reviews still check the bounds, tests, S1–S3 and D4 runs. Earlier drafts went through two Elephant Goldfish rounds (independent AI reviewers, one aspect each):
  - **Round 1:** one combined pass found 1 P0 and 12 P1.
  - **Round 2:** seven parallel reviewers on draft 2 found no P0. Their P1s are resolved, as Appendix B shows.
- **Evidence,** all in this folder (`docs/projects/term/conhost/`):
  - `recon-2026-10-29.md`: how ConPTY and ConDrv work.
  - `spike-1-2026-10-29.md`: spike code, API census, model gaps.
  - `report-2026-10-29.md`: spike results and estimates.
  - `egm-review-1.md` and `egm-review-2.md`: the two review rounds.
- **Spike code:** 1,318 lines of std-only Rust on Beast, Nano's Windows 11 machine (build 26300.9711), at `C:\dev\conhost-recon\rusty_conhost`, outside the repo.

## Summary and decisions requested

We'll replace conhost, the Windows console host behind ConPTY, with our own host running inside rusty_term. A spike already runs interactive cmd and pwsh/PSReadLine on it with no Microsoft host in the process tree. The work is 13 code PRs, plus one PR reserved for fixes. The first milestone, interactive cmd and pwsh in rusty_term behind a config option, lands after PR 5, about 18–24 days in. The full plan is 35–47 days (§10).

| # | Decision | Who | Where | Outcome | Source |
|---|---|---|---|---|---|
| D1 | Build option C: our own host, in-process. This supersedes the "don't roll our own ConDrv" position in `rusty_term/docs/research/conpty-attach-2026-07.md:206-210` | Nano | §1.2 | Approved | Author-reported: Nano in the project thread, 2026-10-29 ("D1. Approved."), recorded on #1222 |
| D2 | Approve the dependency, feature, lockfile and admission changes | Nano / Dottius | §11.1 | Approved by Nano, and co-signed by Dottius for exactly the scope §11.1 enumerates at `a6a98ee7`. It covers nothing beyond that list, and implementation still needs source review and validation. Draft 6 changes no §11.1 row except PR 5's wording | Nano: author-reported, as for D1. Dottius: #1229 re-review comment |
| D3 | Approve the public API additions | Dottius | §11.2 | Accepted at the design-contract level at `a3290ce6`: §11.2 with §7's fairness, control-lane and memory contracts and §5.7's V3. It isn't approval of an implementation; each implementation PR is reviewed against these contracts, and the Ctrl overflow policy and V3 stay explicit limitations | #1229 round-4 and draft-6 review comments |
| D4 | How Windows tests run before review | Dottius | §8.3 | A **successful** run of the existing manual Windows workflow on the exact head of each implementation PR before review. Merge-queue checks are unchanged | #1222 answer relayed by Nano; restated in the #1229 review |
| D5 | Phase B criteria for making our host the default | Nano | §9 | Approved as written in §9.2 | Author-reported, as for D1 |
| D6 | Add `STARTF_USESTDHANDLES` to rusty_term's ConPTY spawn | Nano | §1.1 | Merged as #1227. Both ConPTY tests run again | Verifiable: #1227, and its merge-queue run |

"Author-reported" means the decision was given to the author in the project's chat, not on GitHub, and is restated here; it isn't independently verifiable from the repository.

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
- **Privilege isolation.** The host runs as the user, like conhost. Untrusted input from clients is in scope (§6, §7.4).
- **32-bit (WOW64) clients in the first landing.** S2 checks whether ConDrv hands us the same layouts for them. ARM64 native clients use the 64-bit layouts.

### 1.3 Alternatives considered

| Option | Fixes | Cost | Ongoing burden | Main risk |
|---|---|---|---|---|
| A. Status quo (inbox ConPTY) | nothing | 0 days | Microsoft's bugs, which we can't fix | more drift like #1220 |
| B. Bundle conpty.dll and OpenConsole.exe (#1220) | passthrough, newer fixes | "M+S" in `conpty-attach-2026-07.md:190-280` | tracking and shipping Microsoft binaries | the screen model is still outside our control |
| **C. Our own host, in-process (this design)** | everything in §1.1 | 35–47 days (§10) | maintaining code against undocumented ConDrv and ConsoleControl | Microsoft changes or locks down ConDrv |
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

The engine runs as threads inside the consumer. That means no extra binary and no signal pipe protocol, and a host bug is contained per console (§6.2). What we give up is OS fault isolation.

**Triggers for option D,** a `rusty_conhost.exe` speaking ConPTY's command line, pipe and signal protocol (recon, "ConPTY pipe protocol"):
- a crash that `catch_unwind` cannot contain, seen in practice;
- the §7.4 limits failing to protect the mux server;
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
    Packet(Packet<'a>),   // a READ_IO packet whose variable input the runtime already read (capped, §7.4)
    Input(&'a [u8]),      // VT bytes from the terminal
    Resize(WinSize),
    OutputDrained,        // the reader took bytes out of the queue
    ChildExited,
    HangUp,
    GraceExpired,         // the engine's CTRL_CLOSE grace deadline passed (§7.3)
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

**Payload order on the engine thread:**
1. `wait` delivers the descriptor.
2. The engine calls `read_input` for up to the capped `InputSize`.
3. The engine calls `handle(Packet)`.
4. For each `Reply`, the engine calls `write_output(output)` and then completes the call, either with the next `submit_read` or with `complete_io`.

**Effect rule:** only the engine thread calls `handle` and applies effects, in order. `handle` appends VT to the `OutputQueue` itself, so output order matches the order calls were handled. Ctrl delivery goes to a separate worker. §7 has the full model.

**Tests and adapters:**
- No driver trait is defined. The core is tested by feeding `Event` sequences to `handle` and asserting on effects and queue contents.
- The runtime calls `platform_windows::condrv` directly and is covered by Windows integration tests.
- A trait arrives only if a second real adapter does (option D).

### 3.4 Threads and data flow per console

```text
 terminal (rusty_term)                                   clients (cmd, pwsh, ...)
  write(VT) --> input lane (64 KiB, blocks when full) -----+       |  console API calls
  resize()  --> control lane (flags, latest size wins)     |       v
  hang_up() --> control lane + waker --------------------->+   ConDrv (kernel)
  read()    <-- OutputQueue (cap §7.4) <---- [engine thread] <--> overlapped READ_IO / COMPLETE_IO
            --> control lane: OutputDrained   | owns Console + ConsoleServer + child wait handle
                                              +--> [ctrl worker] bounded, coalesced queue --> ConsoleControl
```

- **Engine thread.** Owns the console. It waits on three things at once: READ_IO completion, the waker and the child's process handle, until the absolute grace deadline (§7.2). Every turn of its loop checks the deadline, the control lane and child exit before it serves one packet and one bounded slice of input.
- **Caller threads.** `write` appends to the input lane and wakes the engine. `resize`, `hang_up` and the reader's `OutputDrained` only set flags in the control lane, which never needs free capacity and is never paused by backpressure (§7.4).
- **Ctrl worker.** Delivers Ctrl events in order from a bounded, coalescing queue, off the engine thread (§7.2, §7.4).
- There is no separate exit-watcher thread: the engine itself waits on the child's handle.

A slow reader therefore stalls clients, as a full ConPTY pipe does, within the caps in §7.4. The mux's contract is "an unread pty blocks the child" (`rusty_mux/crates/server/src/pty/windows.rs:31-33`, `QUEUE = 4` chunks), and this holds that contract.

### 3.5 Modules in `rusty_conhost`

| Module | Job | Spike source |
|---|---|---|
| `msg` | Typed decode/encode of the descriptor, CONNECT info and one struct per supported API. Bounds-checked with `rusty_wire` | dispatch.rs, part (245 lines) |
| `console` | `Console::handle`, the object table, clients and their `ProcessGroupId` | dispatch.rs |
| `input` | INPUT_RECORD queue, VT and win32-input-mode parser, raw passthrough, terminal-reply filter, waiting reads | input.rs (128), vtinput.rs (122) |
| `cooked` | ReadConsole line discipline | readconsole.rs (120) |
| `screen` | Screen over `rusty_term_core::Grid` (scrollback 0) and `AnsiParser`, attribute mapping, read-back | screen.rs (122) |
| `ctrl` | Which pids get which event | ctrl.rs (71) |
| `sched` | The engine's turn order and admission arithmetic as pure functions: what to serve next given the lanes, the readiness snapshot and the clock (§7.2, §7.4). Runs on Linux in tests | new |
| `runtime` (windows) | Engine loop, ctrl worker, admission, panic path, grace deadline (§7) | main.rs (208) |
| `pty` (windows) | `OwnPty`, `OwnPtyMaster` (§4) | new |

The platform mechanism corresponds to the spike's driver.rs (211 lines).

**Layouts and notice:**
- **Owner exception to the clean-room rule.** #1222 asked for owner confirmation before consulting Microsoft's source. Nano gave it in the project thread on 2026-10-29: "Clean room isn't needed against MIT code. The wall exists to avoid copying code you have no right to copy. MIT lets you read, copy and modify with the copyright notice retained … Observer side: reading it to produce behavioral specs is fine." That message is the authority for reading microsoft/terminal here. The MIT licence alone does not settle it. The exception covers MIT-licensed source only.
- Struct layouts are transcribed from microsoft/terminal at commit 09beddc6: `dep/Console/condrv.h`, `conmsgl1.h`, `conmsgl2.h`, `conmsgl3.h` and `src/server/ApiMessage.h`.
- Every file with transcribed layouts or ported logic (`msg`, `cooked`, `input`, `ctrl`, `condrv`) carries the MIT notice, and the crate's `THIRD_PARTY_NOTICES.md` records it (§11.1).
- **Possible reuse:** `rusty_lines` has a private VT key decoder (`rusty_lines/src/lib.rs:1598-1660`). It lacks win32-input-mode, so `input` ports the spike's parser instead.

## 4. Interfaces

### 4.1 `platform_windows::condrv` (PR 1, public API)

```rust
pub struct ConsoleServer { /* server, reference, input/output handles, input event, in-flight READ_IO storage */ }
pub struct Attached {
    pub server: ConsoleServer,
    pub child: WindowsChild,        // built inside platform-windows; from_parts stays pub(crate)
    pub wait: OwnedWinHandle,       // a duplicate of the child's process handle, for the engine's wait
}
pub struct Waker { /* an auto-reset event */ }
pub struct Ready { pub io: Option<IoDescriptor>, pub exited: bool }  // a snapshot; both empty = the waker or the deadline ended the wait
pub struct IoDescriptor { pub id: [u8; 8], pub process: usize, pub object: usize,
                          pub function: u32, pub input_size: u32, pub output_size: u32 }
pub struct IoComplete<'a> { pub id: [u8; 8], pub status: i32, pub information: usize,
                            pub write_back: &'a [u8] }
pub enum CtrlEvent { C, Break, Close }

pub fn probe() -> Result<()> { todo!() }
pub fn spawn_attached(cmd: &Command) -> Result<Attached> { todo!() }
pub fn send_ctrl(pid: u32, event: CtrlEvent) -> Result<()> { todo!() }

impl Waker {
    pub fn new() -> Result<Waker> { todo!() }
    pub fn wake(&self) { todo!() }
}
impl ConsoleServer {
    pub fn submit_read(&mut self, reply: Option<&IoComplete<'_>>) -> Result<()> { todo!() }
    pub fn wait(&mut self, waker: &Waker, exit: Option<&OwnedWinHandle>,
                deadline: Option<Instant>) -> Result<Ready> { todo!() }
    pub fn cancel_read(&mut self) { todo!() }
    pub fn complete_io(&mut self, c: &IoComplete<'_>) -> Result<()> { todo!() }
    pub fn read_input(&mut self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize> { todo!() }
    pub fn write_output(&mut self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()> { todo!() }
    pub fn set_input_available(&mut self, on: bool) -> Result<()> { todo!() }
    pub fn close(self) { todo!() }
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

pub struct OwnPtyMaster { /* Arc<Shared>: input lane, control lane, waker, output state, exit latch */ }
impl platform::pty::PtyMaster for OwnPtyMaster { /* read, write, resize */ }
impl OwnPtyMaster {
    pub fn hang_up(&self) { todo!() }                  // non-blocking, §7.3
    pub fn output_ready(&self) -> bool { todo!() }     // a read would not block
    pub fn exit_waiter(&self) -> ExitWaiter { todo!() }
}
pub struct ExitWaiter { /* Arc of the exit latch */ }
impl ExitWaiter {
    pub fn wait(self) { todo!() }                      // until the direct child exits or the console is torn down
}
```

- `OwnPtyMaster` and `ExitWaiter` are `Send + Sync`. They share only `Shared` (§7.1). They never see `Console` or the server.
- `output_ready` and `exit_waiter` exist because rusty_term's `BackendHandle` needs them (§4.5). They expose two latches the engine already keeps, and no runtime state beyond those.
- Nothing else is exported. No re-exports: consumers name `WindowsChild` through their own direct, Windows-only `platform-windows` dependency (§11.1). We don't add an enum over ConPTY and our host until a consumer needs one. The mux and yirp each get a small local choice in their own PR.

### 4.3 rusty_term_core additions (PR 4, public API)

1. `AnsiParser::pen_colors(&self) -> CellColors`, so GetConsoleScreenBufferInfo `wAttributes` follows VT SGR, reverse and underline included. `Pen` itself stays private (`mod cell` is private, `core/src/lib.rs:29`).
2. `Grid::cell_colors(&self, col: usize, row: usize) -> Option<CellColors>`. It reads the existing provenance tags in `CellStyle` (`core/src/style.rs:29-42`) and the public `ATTR_FG_DEFAULT`/`ATTR_BG_DEFAULT` bits.
   ```rust
   pub struct CellColors { pub fg: ColorRef, pub bg: ColorRef, pub flags: u32 }
   pub enum ColorRef { Default, Palette(u8), Rgb(u32) }
   ```
3. `AnsiParser::palette(&self) -> &[u32; 256]` returns the live palette, including OSC 4 changes (`core/src/parser.rs:125-128`).
4. Fix the stale autowrap doc comment (`core/src/parser.rs:1082`). DECAWM works (`parser.rs:1271-1272`), so `ENABLE_WRAP_AT_EOL_OUTPUT` is emitted as `CSI ?7h` / `CSI ?7l`.

Draft 4's `Grid::resize_clip` is dropped. The host resizes with the existing `Grid::resize` (§5.7).

None of these changes the wire format, so RTCP and RTSN (the mux checkpoint and snapshot formats) don't bump.

### 4.4 Selecting the host

- **rusty_term:** a new config key `[console] host = "conpty" | "own"`, default `"conpty"`. It's parsed in PR 2 and documented in PR 5. It goes under `[console]` rather than `[windows]` so it isn't one letter away from the existing `[window]` section (`rusty_term/src/config.rs:736-788`).
- **rusty_mux server:** there's no config system (`rusty_mux/crates/server/src/lib.rs:49-56`). PR 11 adds a `console_host` field to `Config`, which is set by the env var `RUSTY_CONSOLE_HOST`. It also updates the struct-literal sites (`conn.rs:132`, `state.rs:1019, 1040`, `tests.rs:21`).
- **yirp (`sessionmgr-pty`):** PR 12 adds `PtyOptions::console_host`, using the same env var.
- **No silent fallback.** If `own` fails (`Unsupported`, for example), the error is shown.

### 4.5 rusty_term wiring (PR 5)

A second `BackendHandle` implementation, `OwnBackend`, wraps `(Arc<OwnPtyMaster>, Option<WindowsChild>)`; only the original handle holds the child. The ConPTY backend stays as it is. Unifying the two is a separate follow-up issue after Phase B.

| BackendHandle method | OwnBackend |
|---|---|
| `read_into`, `output_ready` | `OwnPtyMaster::read` and `OwnPtyMaster::output_ready` |
| `write`, `resize` | `OwnPtyMaster::write` / `resize` |
| `try_clone` | Clones the `Arc<OwnPtyMaster>`, with no child. Only the original reaps the child |
| `exit_token` | On the original handle only: `Some(Box::new(move \|\| waiter.wait()))` with `waiter = master.exit_waiter()`, an adapter-owned bridge over the public latch |
| `reap_exit_status` | `WindowsChild::try_wait` |
| Drop | `hang_up`, then hands `wait_shut_down` (up to 6 s) to the backend's reaper thread and returns, so closing a pane never stalls the GUI's event loop. `OwnBackend`'s own Drop joins those waits, so clients still attached are ended (5 s after the hang-up) before the process can exit (#1259); a handle dropped after the backend waits inline. Only if the host hasn't shut down by then is the direct child killed |

`spawn_shell` builds a `platform::process::Command`:
- The program is split off the `shell` string by `CreateProcessW`'s rules (`backend/cmdline.rs`, as built in PR 5), following the `backend/mod.rs:4-8` contract. The rest of the string goes on the child's command line verbatim through `Command::raw_args` (Windows only), so cmd.exe reads what it would under ConPTY; trailing arguments are quoted after it. A batch-file program runs as `CreateProcessW` runs it: `%SystemRoot%\system32\cmd.exe` (lowercase, as `GetSystemDirectoryW` gives it) with the raw text `/c <line>`, the line wrapped in one more pair of quotes when it starts with one (measured on Beast). An unquoted program is the shortest prefix that names a file. What `CreateProcessW` finds before PATH (rusty_term's own directory, the current directory, System32, the Windows directory) is passed as a full path; anything else goes to `Command`'s PATH search.
- `cwd: None` becomes the current directory.
- The second caller, `web_bridge/mod.rs:344`, stays on ConPTY (as built in PR 5).

## 5. Behaviour decisions

### 5.1 Buffer and window
We report a buffer the same size as the window, as ConPTY does. Scrollback belongs to the terminal. SetConsoleScreenBufferSize and SetConsoleWindowInfo succeed only at the window size and return STATUS_INVALID_PARAMETER otherwise. This is divergence V1 (§5.10).

### 5.2 Attributes and colours
- **Writes.** Legacy BGR+intensity maps to SGR: 30–37 and 90–97 for foreground, 40–47 and 100–107 for background. `0x07` on `0x00` maps to 39/49. COMMON_LVB_UNDERSCORE maps to 4, and COMMON_LVB_REVERSE_VIDEO to 7.
- **Index order.** ANSI palette index *i* (0 black, 1 red, 2 green, 3 yellow, 4 blue, 5 magenta, 6 cyan, 7 white, plus 8 for bright) and the Win32 colour nibble (bit 0 blue, bit 1 green, bit 2 red, bit 3 intensity) differ only in the order of bits 0 and 2. The mapping swaps those two bits, in both directions. It is lossless for all 16 values, though the numbers are not identical: ANSI 1 (red) is Win32 4.
- **Reads** use `cell_colors`:
  - `Palette(0..=15)` maps losslessly through that swap.
  - `Default` maps to 7 for the foreground and 0 for the background.
  - `Palette(16..)` and `Rgb` take the nearest of the 16 entries in `palette()`.
  - Reads report what the Grid stores now. A theme change (`retheme`) rewrites matching explicit RGB, so a read after one returns the rewritten value, not the original. The host's Grid lives only in memory and is never saved or restored as RTSN, which doesn't keep palette provenance, so the host never sees the RTSN case where palette cells come back as `Rgb`.
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
1. When PROCESSED_OUTPUT is on and DISABLE_NEWLINE_AUTO_RETURN is off, LF is emitted as CR LF, unless a CR came just before it in the same write.
2. A change to WRAP_AT_EOL_OUTPUT emits `CSI ?7h` or `CSI ?7l`.
3. Without VT processing (either legacy row), WRAP_AT_EOL_OUTPUT wraps as soon as the last column is written, and the host emits CR LF, as conhost's legacy path does. VT output keeps the delayed wrap.
4. Without VT processing, DEL shows as `⌂`. With PROCESSED_OUTPUT off, the C1 controls also show as `?` (conhost `SanitizeUCS2`).
5. Under VT processing, a client's DECRST whose parameters include 9001 or 1004 (`CSI ?9001l`, `CSI ?25;1004l` and the like), or RIS (`ESC c`), is forwarded whole and followed by `CSI ?9001h`, `CSI ?1004h` or both, as conhost does. What it adds counts against the emission step's 64 KiB, as a legacy wrap's CR LF does, so a nested ConPTY that exits can't turn off the input modes the host needs (§5.5).

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
- The host calls the existing `Grid::resize` on its own Grid (scrollback 0) and emits nothing to the terminal. No new resize API goes into rusty_term_core; draft 4's `resize_clip` stays dropped.
- Alternate-screen handling, the stashed primary screen, saved cursors, tab stops, margins and wrap state follow `Grid::resize`'s existing, tested behaviour (`core/src/grid.rs:3656-3760`).
- The host queues a WINDOW_BUFFER_SIZE_EVENT for clients that enabled WINDOW_INPUT.

**The two screens can diverge (V3).** The same algorithm on different histories gives different results. `reflow_history` flattens scrollback plus screen (`core/src/grid.rs:1094-1110`) and drops lines past `scrollback_max` (`grid.rs:1261-1282`). The terminal keeps history and the host keeps none, so a shrink followed by a grow restores lines only on the terminal's side. Dottius's trace:

| Step | Terminal (with scrollback) | Host (scrollback 0) |
|---|---|---|
| 4×3, print `A\r\nB\r\nC` | A / B / C, cursor row 2 | A / B / C, cursor row 2 |
| shrink to 4×2 | B / C (A in scrollback) | B / C (A dropped) |
| grow to 4×3 | A / B / C, cursor row 2 | B / C / blank, cursor row 1 |

After that, the content and the cursor row differ until the screen is redrawn:
- Output the host passes through as VT (text, newlines, relative moves) lands at the terminal's own cursor, so it still looks right on screen.
- Absolute positioning doesn't. SetConsoleCursorPosition(0, 2) followed by WriteConsole("D") makes the host emit `CSI 3;1H D`. The terminal then shows A / B / D, while the host's model, and so ReadConsoleOutput and GetConsoleScreenBufferInfo, say B / C / D.
- Narrowing and widening a wrapped line diverges the same way, because the terminal can rejoin rows from its history and the host can't.

This is recorded as V3 (§5.10). S3 (§9.1) measures how visible it is with cmd, PSReadLine and edit.exe, and is part of the Phase B gate. A synchronization strategy, such as repainting the host's screen to the terminal after every resize, would need its own validation and a design update before this document claims the screens match.

**Cursor resync after resize (#1375).** S3 showed V3 is very visible with PSReadLine in rusty_term's window: PSReadLine reads the cursor from GetConsoleScreenBufferInfo and draws its line with absolute positioning, so its echo and the command's output landed on the host's row, not the terminal's. The host now resyncs the cursor, as ConPTY does (microsoft/terminal GH#18725, `SCREEN_INFORMATION::WaitForConptyCursorPositionToBeSynchronized`):
- A resize that changes the size marks the cursor as possibly wrong. Nothing is emitted yet.
- The next GetConsoleScreenBufferInfo (valid handle, full descriptor) sends DSR 6 to the terminal and is held. Further calls of it are held behind it, and the engine takes no packet until the wait ends, as conhost's API thread blocks. Terminal input keeps flowing.
- The first `CSI row ; col R` in the terminal's input during the wait is the answer, never a key. It moves the host's cursor to that screen position (a position the cursor already reads as keeps its pending wrap), and the held calls are answered from the model. A resize during the wait asks again, since the answer may predate it. So does any output sent after the DSR (a parked write that drains, a cooked read's echo): the CPR is the terminal's cursor where it parsed the DSR, and the model has moved on since.
- With no answer in 500 ms, counted from the first DSR however often the terminal is asked again, the held calls are answered from the model, as before. A resize during that wait leaves the cursor marked, so the next call asks again.
- A CPR outside a wait is input, as before.

Only the cursor is resynced. The contents can still differ (ReadConsoleOutput), as under ConPTY. `v3_the_cursor_resyncs_from_the_terminal_after_a_resize` pins it at core level, and `own_pty.rs`'s `a_client_positioning_from_the_cursor_follows_the_terminal_after_a_resize` on Windows.

**Regression tests (PR 4, Linux, core level).** Each test runs the same VT through a host Grid (scrollback 0) and a terminal-side Grid (with scrollback), resizes both and asserts the documented result for each, so V3 is pinned as an expected difference rather than discovered later:
- shrink then grow, using the trace above;
- narrow then widen with a wrapped line, primary and alternate screen;
- each of those followed by a cursor-positioned write (SetConsoleCursorPosition, then WriteConsole) and a relative write, asserting the host model, the VT emitted and the terminal-side result.

### 5.8 Lifetime
**EOF** comes when the direct child exits and the queue drains. That matches today's `spawn_exit_watcher` (`platform-windows/src/sys/pty.rs:624-643`). Grandchildren (`start /b`) don't keep the console alive.

**Shutdown:** §7.3 is normative. In short, on `ChildExited` or `HangUp` the core sends `Ctrl{Close}` to the remaining clients through the ctrl worker and the engine arms a 2 s deadline. 2 s matches today's Drop drain bound (`sys/pty.rs:691, 727`). On the last DISCONNECT or at the deadline, the engine cancels and drains its read, closes the server and emits `Eof`. Clients still attached at the deadline are ended 5 s after CTRL_CLOSE, once the server has closed (#1259, §7.3).

**`hang_up` contract:** `hang_up` returns at once and never calls `ConsoleControl` itself. Today's `hang_up` blocks only on a full pipe (`platform-windows/src/pty.rs:147-160`). The mux's `GRACE` = 1 s kill (`rusty_mux/crates/server/src/pty/mod.rs:29`) still works on top of it.

**Before PR 8, and if S1 shows CTRL_CLOSE fails:** the Ctrl step is skipped. The server closes at once, and Drop kills the direct child after 2 s (§4.5).

### 5.9 Code pages
- **PR 3:** GetConsoleCP and GetConsoleOutputCP report 65001. That is divergence V4: ConPTY reports the OEM code page, but every consumer here speaks UTF-8. SetConsoleCP(65001) succeeds. Other code pages get STATUS_NOT_SUPPORTED, and the `A` APIs treat their bytes as UTF-8.
- **PR 9, before Phase B:** any installed code page is accepted. Conversion goes through a `codec` function supplied in `ConsoleConfig`, the same way `key_for` is. It is `MultiByteToWideChar` on Windows and a table for 437/850 in tests, so `chcp 850/932` works.
- **As built (PR 9):** the console keeps conhost's two code pages. The input one (SetConsoleCP) covers ANSI ReadConsole, raw reads, ANSI GetConsoleInput (one record per code-page byte) and the `A` title calls; the output one (SetConsoleOutputCP) covers WriteConsoleA and raw writes. Both start at 65001 (V4), and SetConsoleCP takes any code page `IsValidCodePage` accepts, else `E_INVALIDARG`, as conhost does. The codec is a `Codec` of three functions passed to `Console::new` next to `key_for`: `condrv::{is_valid_code_page, decode, encode}` on Windows, tables for 437, 850 and a slice of 932 in tests. 65001 never reaches the codec. A DBCS lead byte that ends a WriteConsoleA waits for the next write, which drops it if its first byte is a control, as conhost's `WriteConsoleDbcsLeadByte` does; a change of output code page drops it too (V14). conhost finds lead bytes with `GetCPInfo`; we ask the codec instead (a lead byte and `A` decode to one unit), which keeps the admitted set at the three calls of §11.1 (V14). Only the output code page needs them, so SetConsoleCP only checks the code page. A parked `A` write keeps its bytes, so it retains no more than its packet; each slice decodes the whole write afresh and emits the next units, so no cut splits a multibyte sequence or loses a stateful code page's shift state (UTF-7, ISO-2022). Under 65001 an unfinished UTF-8 sequence at the end of an `A` write waits in the console for the next one, and any nonempty write in another code page drops it, even a lone lead byte that shows nothing, as conhost resets its `u8State`; CP 54936 and UTF-7 writes are checked on Windows against one native decode of the whole write, parked or not. NumBytes reports every byte as written.

### 5.10 Known divergences from conhost

| # | Behaviour | conhost | We | Why |
|---|---|---|---|---|
| V1 | Buffer or window other than the terminal's size | SetConsoleScreenBufferSize grows the buffer (e.g. 9001 rows); under ConPTY SetConsoleWindowInfo resizes the buffer to the window asked for; GetLargestConsoleWindowSize reports what the display could hold | SetConsoleScreenBufferSize and SetConsoleWindowInfo succeed only at the window's size (`E_INVALIDARG` otherwise); GetLargestConsoleWindowSize reports the window | scrollback belongs to the terminal, and the terminal owns the size: a buffer the host resized alone would wrap text where the terminal doesn't |
| V2 | SetConsoleScreenBufferInfoEx colours | changes the palette | no-op | no target program needs it |
| V3 | Content after resize | one buffer, so one history | the host's model and the terminal's screen can differ after a shrink then grow, or a narrow then widen, until a redraw. The cursor is resynced from the terminal (DSR 6) before the next GetConsoleScreenBufferInfo, as ConPTY does, so absolute positioning lands on the terminal's rows (§5.7, #1375) | the host keeps no scrollback, the terminal does |
| V4 | Startup code pages, input and output | OEM (e.g. 437) | 65001; `chcp` changes both from PR 9 | consumers are UTF-8 |
| V5 | GetConsoleWindow | a hidden pseudo-window | NULL | add a message-only window if a program in the census needs it; conhost's source names cygwin, which fails silently on NULL, so cygwin and MSYS2 (git-bash tools) are in the census |
| V6 | Multiple screen buffers (CREATE_OBJECT type 3, SetConsoleActiveScreenBuffer) | supported | STATUS_NOT_IMPLEMENTED | no target program uses them (edit.exe uses `?1049h`); build when the unknown-API log shows a real caller |
| V7 | GenerateConsoleCtrlEvent for a group no client is in | falls back to the caller's parent process | `E_INVALIDARG`, nothing sent | the fallback serves console-less callers; our clients always carry their group |
| V8 | Synthesized key records (plain VT input, not win32-input-mode) | scan codes from the layout; mouse reports become mouse records | scan code 0; mouse reports reach VT-input clients only | no target program reads scan codes or mouse records from a VT terminal; win32-input-mode carries both exactly |
| V9 | Closing after CTRL_CLOSE | no host deadline: serves until the last DISCONNECT; Windows ends a handler at about 5 s | the server closes at the last DISCONNECT or 2 s after the first hang-up or child exit (§7.3), so a handler taking 2–5 s loses its console | teardown stays bounded, matching today's 2 s Drop drain (§5.8); a client that ignores the closed console's errors can't hold it open (S1), and one still attached at the deadline is ended 5 s after CTRL_CLOSE with `STATUS_CONTROL_C_EXIT`, the code CTRL_CLOSE ends it with (#1259). conhost never ends a client itself; 5 s is Windows' own bound on a CTRL_CLOSE handler, so a handler Windows would let finish isn't cut short, but one whose CTRL_CLOSE was delayed behind another client's handler is ended with the rest (approved by Nano 2026-10-31) |
| V10 | A client connecting during the close | CTRL_CLOSE is sent once (`_closeEventSent`), so a late client gets none | a late client gets CTRL_CLOSE once its CONNECT reply has gone out | without it a late `ping -t`-like client outlives the console (S1). Since #1259 a late client is also ended 5 s after the first CTRL_CLOSE, whatever handler time it has left (one connecting at 4.9 s gets about 0.1 s) |
| V11 | DSR 5, CPR and DECXCPR from a client (§5.6) | under ConPTY never answered: the query goes to the terminal, whose CPR comes back as input | answered from the host's Grid, and the query is kept from the terminal | the answer then agrees with GetConsoleScreenBufferInfo even after a resize (V3), and a CPR never has to be told apart from a modified F3 key on input |
| V12 | Input availability while an earlier read is held on echo or the 1 MiB reply budget | count describes buffered records | count, the input event and a NOWAIT peek report no available records until the owed reads resume | those records belong to the earlier reads; later readers must not take their input |
| V13 | GetConsoleLangId | answers for an East Asian output code page when the system's ANSI code page is East Asian too | always fails (`STATUS_NOT_SUPPORTED`) | the answer needs the system's ANSI code page, which we don't read; clients fall back to their own UI language, as they do under conhost on every other system |
| V14 | WriteConsoleA's kept DBCS lead byte | lead bytes from `GetCPInfo`; the kept byte survives a change of output code page | lead bytes probed through the codec, so they may differ where `GetCPInfo` lists none for a multibyte code page (possibly GB 18030 and EUC-JP: we keep a two-byte lead, conhost keeps nothing); a change of output code page drops the kept byte | no fourth Win32 admission; a byte kept across a code-page change would pair with a byte of the new one |
| V15 | The hosting process ends abruptly (the mux server killed or crashing, or its console window closed) | OpenConsole is its own process: the broken pipes make it send CTRL_CLOSE, and Windows ends the handlers at about 5 s | the host runs inside the server, so it ends with it and nothing runs: clients still attached get no CTRL_CLOSE and are never ended | the console stays bounded and in-process (D1). On Beast a killed server's cmd ended within 3 s, but a `start /b` survivor can outlive it. A clean stop does not share this: the mux server's `serve_until` (Ctrl+C or Ctrl+Break for the binary) and any end of `serve`/`serve_udp` end every block's program, hang up every console we host and wait, with one 6 s deadline for them all, until each has shut down: closed, and the end of each client still attached tried 5 s after CTRL_CLOSE (`wait_shut_down`). A client the host could not open or end may still run. Past the deadline the server logs and exits anyway. Closing the server's own console window counts as abrupt, since Windows ends a process once its CTRL_CLOSE handler returns. yirp (#1290) likewise awaits a bounded shutdown on normal exit and abandon (`shut_down_terminal`) |

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

### 6.1 Limits (security and availability)

| Limit | Value | Where it comes from | On breach |
|---|---|---|---|
| Variable input read per packet | 64 KiB for every API. A WriteConsole or raw write past it is streamed: read and emitted 64 KiB at a time (§7.4). A read API's reply may use its declared output capacity, up to 1 MiB (§7.4) | the spike's largest real request was 16,404 bytes (cmd ReadConsole); PR 14's bench found real writes past it (one `WriteFile` of a big buffer, Rust's stdout) | other APIs: STATUS_INVALID_PARAMETER |
| Output queue, input lane, waiting calls, parked payload, Ctrl queue, client and object tables, title | §7.4 | the mux's 4-chunk contract; the spike's census | §7.4 |
| Input queue | 4,096 records | about 1 s of fast typing or paste at 4K keys/s | oldest dropped, logged once |

Further robustness points:
- **Decoding:** client sizes, offsets and ApiNumbers are never trusted before decoding (`msg`, fuzzed in PR 13).
- **Handle inheritance:** the handle list is limited to in, out and err (§2 step 4).
- **Logs:** capture fixtures are sanitised (§8.2), so logs and fixtures never carry CONNECT's app path, title, current directory or environment.

### 6.2 Panics
- **Containment.** On the engine thread, `handle` runs inside `catch_unwind(AssertUnwindSafe(|| console.handle(ev, &mut q)))`.
- **On a panic,** the console is marked dead, and every pending call completes with STATUS_UNSUCCESSFUL. After that, `CloseServer` and `Eof` run.
- **Before each `handle`,** the runtime checks the dead flag. New packets on a dead console are answered STATUS_UNSUCCESSFUL straight away.
- **Poisoned `Shared` locks** are taken with `unwrap_or_else(PoisonError::into_inner)` (`platform-windows/src/pty.rs:67`).
- **Library code** has no `unwrap`.

## 7. Concurrency, shutdown and memory

This section is normative for PRs 2, 3 and 8. Draft 6 adds the fair turn order and absolute deadline (§7.2), the control lane and the bounded Ctrl queue (§7.3–§7.4), answering round 4. Draft 5 replaced draft 4's "mutex, then run effects after unlocking" rule, which let two threads apply `SetInputAvailable` out of order (round 3, item 2).

### 7.1 Ownership

| Owner | Holds | Never touches |
|---|---|---|
| **Engine thread**, one per console | `Console`, the `ConsoleServer` (by value, in an `Option`), the in-flight READ_IO storage inside it, the child's wait handle, the grace deadline | caller-thread stacks |
| **Ctrl worker**, one per console | the receiving end of the bounded Ctrl queue of `(pid, CtrlEvent)` (§7.4) | `Console`, `ConsoleServer`, any handle |
| **Caller threads** (`read`, `write`, `resize`, `hang_up`, Drop) | an `Arc<Shared>`: the input lane, the control lane, the `Waker`, the output state and its `Condvar`, the exit latch | `Console`, `ConsoleServer` |

- `Console::handle` runs only on the engine thread, so there is no `Mutex<Console>`.
- `Shared`'s locks are held only to push or pop. No OS call runs under any of them.
- **Two lanes.** The *input lane* holds terminal VT bytes, up to 64 KiB, and `write` blocks when it's full. The *control lane* is a fixed-size struct of coalesced flags: `hang_up`, `detached`, `output_drained`, and `resize: Option<WinSize>` (the latest size wins). Setting a flag never needs free capacity and never waits, and nothing pauses the control lane.

### 7.2 The engine loop and effect order

The loop's order is a pure function in `sched`, so the tests below run on Linux with a fake clock. `deadline` is an absolute `Instant`, armed once, and no wake or event moves it.

```text
loop {
    now = Instant::now()
    1. if deadline is armed and now >= deadline: handle(GraceExpired), disarm
    2. take every control flag set since the last turn:
         hang_up → handle(HangUp); resize → handle(Resize); output_drained → handle(OutputDrained)
    3. if exited and not yet handled: handle(ChildExited)
    4. if the output queue is ≤ 80 KiB (§7.4): take at most 16 KiB from the input lane, handle(Input)
    5. if no READ_IO is in flight and admission allows (§7.4): server.submit_read(pending_reply)
    6. ready = server.wait(&waker, exit_unless_handled, until)
         until = now when steps 2–4 left work they could still do, otherwise the deadline
    7. if ready.exited: note it for step 3
    8. if let Some(desc) = ready.io: read_input (capped), handle(Packet), apply effects
}
```

Effects are applied after each `handle` call, in the order the core returned them.

**Fairness.** `wait` has no priority order. It returns a snapshot of everything ready when it wakes: the completed read and whether the child has exited (§11.2.1). The loop then serves sources in a fixed order on every turn:
- **One packet per turn.** Steps 1–3 run between any two packets, so an always-ready stream of client calls can't starve them.
- **Bounded input per turn.** Continuous terminal input can't starve packets or control, because each turn takes at most 16 KiB.
- **Bounded turn length.** A turn handles at most one packet, emitting at most one 64 KiB slice (§7.4), plus 16 KiB of input and the constant-size control lane. So `hang_up`, a resize, child exit and the grace deadline are each served within one turn of becoming true, whatever the clients and the terminal are doing.
- **Wake-ups.** The waker only ends a `wait`. The engine reads the control lane on every turn, so a wake that lands while the engine is busy isn't lost.

- **One executor.** The engine applies the effects of one `handle` call, in the order the core returned them, before it calls `handle` again. `SetInputAvailable` values therefore reach the OS in the order the core produced them. The last value applied is always the core's current truth. Draft 4's false→true reordering, and the lost wakeup when the order is reversed, cannot happen.
- **No-op skipping.** The engine remembers the last value it applied and skips an identical one.
- **Stale effects.** The core completes each request id exactly once. That is a core invariant with its own test. After `CloseServer`, the engine drops any later `Reply` or `Complete`, counting them in one debug log line, because their clients are already disconnected.
- **Ctrl delivery is never on the engine thread.** `Effect::Ctrl` is pushed onto the ctrl worker's queue, which is bounded and coalescing (§7.4) and never blocks the engine. The worker calls `condrv::send_ctrl` in queue order. While it does, the engine keeps serving the driver, so a handler's own WriteConsole is answered. That removes the deadlock without holding any lock across delivery. Ctrl effects can reach the OS after later driver effects. That is intended, and safe: a Ctrl event carries no state the driver needs.

**Tests (deterministic, Linux, `sched` with a fake clock and a scripted readiness source):**
- **always-ready IO:** every `wait` returns a packet. `hang_up` set at turn *k* is handled at turn *k*. Child exit reported at turn *k* is handled by turn *k* + 1. The grace deadline expires on the first turn at or after it, while packets keep arriving.
- **continuous commands:** the input lane is refilled to 64 KiB before every turn. Each turn still serves one packet and the control lane, and takes at most 16 KiB of input.
- **both at once:** always-ready IO and continuous input together, with the same three progress assertions.
- **deadline is absolute:** wakes and packets arriving every millisecond during the grace don't delay `GraceExpired` past the first turn at or after the deadline.

**Tests (deterministic, Linux, core level).** Each interleaving is an ordering of `Event`s, because the engine serialises them. After every event the test asserts the invariant: the last `SetInputAvailable` emitted equals "the input queue is non-empty". It runs:
- enqueue then drain, and drain then enqueue, each with a waiting read before, during and after;
- `HangUp` arriving between a `Packet` and its completion;
- `ChildExited` racing a pending cooked read;
- a second `HangUp`.

**Tests (Windows integration).** The probe binary runs a client that loops `PeekConsoleInput`/`ReadConsoleInput` while the test writes input from the terminal side. At quiescence the input event is signalled if and only if records are queued. A shutdown test calls `hang_up` while a ReadConsole is pending and while a write is parked, and checks that both calls complete and `read` reaches `Ok(0)` once.

### 7.3 Shutdown and cancellation

- **`hang_up`** sets the control lane's `hang_up` flag (a second call finds it set and returns at once), wakes the engine and returns. It needs no input-lane capacity. It takes no lock that is held across an OS call, so it never waits on a client handler.
- **Drop of the last `OwnPtyMaster`** does the same, and also marks the console detached. The engine then tears down, frees its own state and exits. Drop never joins a thread.
- **Engine on `HangUp` or `ChildExited`.** The core emits `Ctrl{Close}` for every client, newest first as Windows dispatches them (onto the Ctrl queue; a Ctrl effect that comes with a reply is queued once that reply is sent), and the engine arms the deadline at `now + 2 s` once. A later `HangUp` or `ChildExited` doesn't move it. During the grace the engine keeps serving the driver.
- **Client processes (#1259).** The core emits `Joined{pid, tid}` before a successful CONNECT's reply, `tid` being the descriptor's `object`, the thread the CONNECT came from (conhost reads it the same way, `IoDispatchers.cpp:431`). The engine opens the client with `condrv::ClientProcess::open(pid, tid)`. One handle is held per pid, bounded by the client limit (§6.1), and is released on `Left{pid}`, when the pid's last connection goes. ConDrv does **not** keep a waiting client referenced (Beast, 2026-10-31, author-reported): a client ended from outside while its read CONNECT is unserved, with no other handle to it, frees its pid at once, so the pid alone may name another process by the time the engine opens it. `open` therefore opens the thread first (`OpenThread(THREAD_QUERY_LIMITED_INFORMATION)`), requires `GetProcessIdOfThread` to be `pid`, and only then opens the process; the thread handle references its process, so the pid can't be freed between the check and the open. The engine holds the client only once its CONNECT reply, completed on its own, is delivered. That makes the identity authoritative rather than probable: a thread can't finish exiting while its CONNECT is pending, and ConDrv aborts a CONNECT whose thread dies, both before the host reads it (it is never delivered) and after (the reply fails with ERROR_OPERATION_ABORTED; both Beast, 2026-10-31, `tests/condrv.rs`). So a delivered reply proves the CONNECT thread was alive when the engine opened `tid`, and a live thread's id names only it, even if a new process had reused both ids. It fails closed: a thread that is gone or another process's, a reply that fails, or a client that can't be opened (a protected process) is never held, so never ended; it is logged and still gets CTRL_CLOSE. These calls are a D2 admission, co-signed by Nano on 2026-10-31 and proposed on #1286 for Dottius's.
- **Engine on the last DISCONNECT or on `GraceExpired`.** At `GraceExpired` the core first emits `Kill{pid}` for every process still attached, and the engine moves those handles aside. Once the server is closed and every waiter released, the engine thread sleeps until 5 s after CTRL_CLOSE (`KILL_AFTER`, Windows' own handler bound) and calls `ClientProcess::terminate` on each, which ends a client still running with `STATUS_CONTROL_C_EXIT`, leaves one that exited alone, and doesn't wait. A failure is logged. Nothing waits on that thread. After the last DISCONNECT no client is left, so nothing is ended. The core then emits `CloseServer`, and the engine:
  1. calls `cancel_read` if a READ_IO is in flight;
  2. keeps calling `wait` until that read's completion arrives, whatever it is: success, cancelled or error. CancelIoEx only requests cancellation, and the OVERLAPPED block and buffer can't be freed or reused before completion. A descriptor that completed successfully in the race is answered `STATUS_UNSUCCESSFUL` with `complete_io`, best effort;
  3. calls `server.take().map(ConsoleServer::close)`, so `CloseServer` reaches the consuming `close(self)`;
  4. emits `Eof`.
- **Bounded teardown.** If the cancelled read hasn't completed within 1 s, the engine leaks that read's storage on purpose, logs it once and closes the handle. It never frees memory the kernel may still write. ConDrv completes cancelled reads at once in the spike, so this path is a guard, not an expected case.
- **After a host panic or an engine error** (§6.2) the engine closes without ending any client, and sends no CTRL_CLOSE unless the close had already begun. Clients then see their console calls fail (error 233); the direct child is still killed by the consumer's Drop (§4.5), but a grandchild that ignores the errors runs on, as in S1. If the host process itself dies, nothing in it can act at all.
- **A stalled Ctrl worker** can't delay teardown. The deadline belongs to the engine, and the worker owns no handles, only pids. If `ConsoleControl` hangs, the worker is abandoned and exits once its call returns and it sees the closed channel.
- **No guarantee of `Interrupted`.** A cancelled read may complete with success, `STATUS_CANCELLED` or another error. The engine handles every outcome the same way.
- **EOF exactly once.** `Eof` sets a latch in the output state, and only the engine sets it. Once the queue is empty and the latch is set, every `read` returns `Ok(0)`, repeatedly. The exit latch that `ExitWaiter` waits on is set exactly once, by the first `Ready::exited` or by the engine's own exit, whichever comes first.

**Tests:**
- cancel with no read in flight (a no-op);
- cancel racing a successful completion (a Windows test that sends a client call at the moment of hang-up, repeated 1,000 times);
- effects arriving after `CloseServer`;
- repeated `hang_up`;
- Drop during the grace period;
- exactly one transition to EOF, with `read` called from two threads.

### 7.4 Memory bound

**Named buffers.** Each buffer below has a cap and an admission rule. Together they bound the bytes that client calls and terminal input can make the host hold:

| Store | Cap | Admission when full |
|---|---|---|
| Output queue, client output | 64 KiB | a call that produces VT (WriteConsole, RAW_WRITE, WriteConsoleOutput(+String), FillConsoleOutput, ScrollConsoleScreenBuffer, SetConsoleCursorPosition, SetConsoleTextAttribute) is admitted only while the queue is ≤ 64 KiB; otherwise it is parked |
| Output queue, host reserve | 16 KiB above that | host-originated output (cooked-read echo, `^C`, the startup preamble) may use it. Past 80 KiB the engine stops taking bytes from the input lane (§7.2 step 4). Echo is checked before every character, and Escape erases one character per step, so a cooked read stops there with its input still queued and goes on when the reader drains. The control lane is still served |
| One emission step | 64 KiB | a call whose output would be larger (a 64 KiB WriteConsole whose ESC bytes expand to U+2190, or a full-screen WriteConsoleOutput) emits in slices of at most 64 KiB, and is parked with its progress between slices. It completes only when fully emitted |
| Input lane (terminal input) | 64 KiB of pending bytes | `PtyMaster::write` blocks until the engine drains it, as a pty write blocks when the child stops reading |
| Admitted terminal slice | 16 KiB of bytes, plus one decoded sequence (at most 4,097 raw bytes or a wrapped key's records) | no later slice is taken until this remainder is consumed. Decode incrementally and serve a waiting cooked read between records, before processing a later Ctrl+C. Echo and reply-budget holds retain the remainder instead of overflowing the record queue |
| Control lane | fixed size: four flags and one `WinSize` | never full: setting a flag that is already set is a no-op |
| Input records | 4,096 for terminal input | terminal input arriving with 4,096 or more queued drops the oldest record first, whatever its source, logged once: it never grows the queue past 4,096 or its current length, and can displace a client's written records. WriteConsoleInput is never refused or trimmed (Nano, 2026-11-01, #1335), so a client that writes and never reads grows the queue by 20 bytes a record; the total below excludes those |
| Unfinished input escape sequence | 4,096 bytes | given up as an unknown sequence and dropped |
| Cooked line | 8,192 UTF-16 units, or the read's initial data if longer. **Reserved** in the parked payload when the read starts waiting, so it is within the 64 KiB per call | further typing is ignored |
| Rest of finished lines kept per handle for the next read | 512 Ki units (1 MiB) across all handles | the rest of the line that would pass it is dropped, logged once |
| ANSI read caches (a split UTF-8 character, records expanded past the count) | one character: 3 bytes, 2 records | never full: each read empties them before refilling |
| Waiting calls (reads and parked writes together) | 64 | READ_IO is submitted only while fewer than 64 calls wait, so further calls wait in the kernel and cost the host nothing |
| Payload retained by parked calls | 1 MiB in total, **reserved before a packet is accepted** | READ_IO is submitted only while `parked + 64 KiB ≤ 1 MiB`, 64 KiB being the most variable input one packet can bring. A packet that then arrives always fits, so the cap is never overshot |
| Variable input read per packet | 64 KiB | a WriteConsole or raw write is **streamed**: the engine reads its first 64 KiB with the packet and each next slice (`Effect::Fetch`, `READ_INPUT` at an offset) only once the last is emitted, so it retains one slice at most. The engine reads at most one slice per turn, after the control lane, the child's exit and the deadline (§7.2), and a due slice keeps the wait from blocking. The call's first-of-write state (an LF's added CR) is the call's, not each slice's. ANSI text (a raw write or WriteConsoleA) under a stateful output code page (UTF-7, ISO-2022 50220–50229, HZ 52936) is not streamed but refused as before, since a slice boundary would lose its shift state; W text always streams. Its reply counts every byte. Until it completes, no READ_IO is sent, so the call stays whole as under conhost's lock and its slices decode in order (a split UTF-8 or DBCS character, or a surrogate pair, is carried to the next slice). A slice the driver can't read fails the call with `STATUS_UNSUCCESSFUL`. Any other API: `STATUS_INVALID_PARAMETER` |
| Answers to the host's own queries (§5.6) | they count against the 64 KiB client budget, so one turn builds at most 64 KiB plus one emission step's answers (64 KiB of 5-byte queries, 11-byte answers: about 141 KiB) | the next write waits for the reader to drain, as if its answers were output; when no output is queued to drain, the next engine turn goes on with it (`Turn::pump_writes`) |
| Read reply | the read's declared capacity, up to 1 MiB, and 1 MiB across the replies one call builds | a waiting read holds only its descriptor. Its reply is built when it completes and freed once written. A call that reaches the 1 MiB stops serving waiting reads; the next turns serve the rest before taking new input, without waiting for more |
| Ctrl queue | 256 entries | coalescing: an entry equal to one already queued is dropped, and a queued `Close` for a pid drops later `C` or `Break` for it. Past 256 the new entry is dropped and logged once. Pushing never blocks the engine. Teardown doesn't depend on queued entries, because the engine closes the server at the deadline (§7.3) |

The output queue therefore never exceeds 64 + 16 + 64 = 144 KiB. The named buffers above total at most 2,400 KiB, about 2.3 MiB, per console: 144 KiB of output, 64 KiB of input lane, 80 KiB of input records (4,096 × 20 B), 1 MiB of parked payload, one 64 KiB packet read and one 1 MiB read reply; the Ctrl queue adds about 2 KiB, and kept line rests at most 1 MiB more, 3.3 MiB in all. That figure covers only these buffers. The stores in the next table have their own limits and aren't in it.

**Other client-controlled stores and their limits:**

| Store | Limit | On breach |
|---|---|---|
| Screen (`Grid` and `AnsiParser`) | the terminal's window size, scrollback 0. Only `resize` from the terminal changes it. Clients can't grow it, because the buffer is the window (V1), and SetConsoleScreenBufferSize to any other size fails | `STATUS_INVALID_PARAMETER` |
| Connected clients | 256 | CONNECT fails with `STATUS_NO_MEMORY`, logged once |
| Objects per client | 64 | CREATE_OBJECT fails with `STATUS_NO_MEMORY` |
| Title | 4,096 UTF-16 units | truncated, as conhost does |

Any store a later PR adds names its cap in one of these tables in the same PR.

**Tests (stalled reader).** For WriteConsole, RAW_WRITE, FillConsoleOutput, ScrollConsoleScreenBuffer, WriteConsoleOutput, WriteConsoleOutputString, and cooked echo under sustained terminal input, the test never reads. It asserts:
- the output queue's high-water mark is ≤ 144 KiB;
- parked payload is ≤ 1 MiB;
- READ_IO submission stops once the waiting-call cap is reached;
- draining the reader completes every parked call in order.

**Tests (backpressure and control):**
- **input and output full together:** the input lane is at 64 KiB with `write` blocked, and the output queue is past 80 KiB. `hang_up` is still handled on the next turn, and teardown reaches `Eof`. In a second run the reader drains instead: `OutputDrained` is handled on the next turn, input resumes, and the blocked `write` returns.
- **drain or `HangUp` behind queued input:** each control flag set while 64 KiB of input is queued is handled before any of that input.
- **repeated Ctrl with a stalled worker:** the worker blocks forever, and clients send 10,000 GenerateConsoleCtrlEvent calls plus input Ctrl+C. The queue stays ≤ 256 entries, duplicates coalesce, the engine keeps serving packets, and teardown completes at the deadline.
- **capacity-boundary packets:** parked payload at exactly 1 MiB − 64 KiB, then a 64 KiB packet: accepted, and the next READ_IO is held back. 63 waiting calls, then one more: accepted, then held back. A non-write packet one byte over the per-packet limit gets `STATUS_INVALID_PARAMETER`; a raw write or WriteConsole(W) of several slices completes whole, with the output queue within its bound and every character intact. The 257th client and the 65th object fail as the table says.

## 8. Testing

### 8.1 Test layers

| Layer | Runs on | Oracle | First PR |
|---|---|---|---|
| Core: `Event` sequences into `handle` | Linux PR CI | golden effects and golden VT per API; the §5.4–§5.6 tables; effect-order, backpressure, panic and resize scenarios | 2 |
| condrv integration | Windows | the probe's own observed result (exit code, logged event) | 1 |
| Parity: each probe run under ConPTY and under our host, fields compared | Windows | inbox conhost's answer, minus the §5.10 divergences | 2, and each API PR adds probes |
| Concurrency and memory: §7.2–§7.4 test lists | Linux (core), Windows (runtime) | the invariants stated there | 2, 3, 6, 7, 8 |
| Fuzz: `msg` decode and packet sequences | nightly | no panic, every effect within limits | 13 |
| Manual gate | Beast plus one older build | see below | each milestone |

- **Probes** are the `conhost_probe` example in rusty_conhost (`examples/conhost_probe/`): one binary with a subcommand per API, each a real client calling through kernelbase. Tests spawn it from the target directory. No extra workspace member is needed. The console calls it makes are admitted in their own block of `win32_surface` (D2, asked on the suite's PR).
- **The parity suite** is `own_pty.rs`'s `parity_suite_agrees_with_conpty`, the first Phase B criterion (§9.2). It runs every probe under ConPTY and our host and compares the result lines. A difference must be a §5.10 divergence (`DIVERGENCES`, with our exact answer) or a tracked gap (`GAPS`, with its issue); a listed gap that starts to agree fails until it is taken off. "The parity suite passes" in §9.2 means it passes with `GAPS` empty. `cargo test --test own_pty` builds no examples, so the test refuses a probe older than its sources.
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

**Decided (D4, Dottius, 2026-10-29):** before an implementation PR is put up for review, a **successful** run of the existing manual Windows CI workflow (`workflow_dispatch` on `ci.yml`, a full sweep including `windows-latest`, `.github/scripts/ci_plan.py:111-151`) on the PR branch at the exact head commit is required. A run in which the Windows test jobs were skipped doesn't count. A docs-only PR's CI is not ConDrv validation. The PR body links that run and names its commit. A push after the run means a new run before re-review. The normal merge-queue checks stay required. This is not a CI change. A Beast test log may be attached as extra evidence, never instead of the run.

## 9. Rollout

### 9.1 Spikes still owed

| Spike | Question | Where | Blocks | Estimate |
|---|---|---|---|---|
| S1 | Do CTRL_CLOSE and Ctrl+Break work through ConsoleControl? What do clients do when the server closes? | Beast | PR 8 | 0.5 days |
| S2 | ConDrv and ConsoleControl on a GitHub `windows-latest` runner (build number), WOW64 layouts, and the oldest machine available | runner via `workflow_dispatch`, Beast | PR 1 acceptance | 0.5 days |
| S3 | How visible is V3, the resize divergence (§5.7), with cmd, PSReadLine and edit.exe? Does a repaint after resize remove it? | Beast | Phase B | 0.5 days |

S2's verified runner result at PR 1 head `208a5b5c` was Windows Server
2025, build 10.0.26100, on image `windows-2025-vs2026` version
`20261021.278.1`; the ConDrv tests and Windows gates passed. The Beast and
WOW64 observations are author-reported rather than independently verified.
The older-machine check remains owed, as does a conclusive Ctrl+Break probe.

S1 ran on Beast (Windows 10.0.26300.9818) on 2026-10-30, at main
`1a0a328b`, with clients hosted by `OwnPty` and `send_ctrl` called off the
serving thread. Results are author-reported:
- **CTRL_CLOSE works.** The handler runs and its WriteConsole is served
  while it runs. Windows ends the process about 3 ms after the handler
  returns, even when it returns TRUE (exit `0xC000013A`), and ends a
  handler still running at about 5 s, though we, not csrss, sent the
  event. `send_ctrl` blocks for as long as the handler runs. cmd idle at
  its prompt exits in 2 ms; `ping` prints its statistics, then exits.
- **Ctrl+Break works.** A handler sees event 1 and keeps running;
  `ping -t` prints its statistics and goes on. A client ignoring Ctrl+C
  still gets Break.
- **On server close** no Ctrl event is delivered and every console call
  fails with error 233 (ERROR_PIPE_NOT_CONNECTED). cmd exits, but a
  client that ignores the error (`ping -t`) runs on indefinitely.
- **So PR 8** sends CTRL_CLOSE before closing, keeps serving until the
  last DISCONNECT (closing on the direct child's exit cut off a
  grandchild's last output in S1), and its ctrl worker still delivers
  queued CTRL_CLOSE after the server closes, leaving Windows' 5 s rule to
  end slow handlers. PR 8 killed no client: that needed a process
  handle the host didn't own. #1259 adds one per client, opened at
  CONNECT, and ends clients still attached at the deadline 5 s after
  CTRL_CLOSE (§7.3).

WOW64 (author-reported on Beast): 32-bit clients send the same 64-bit
packet layouts, so there is no 32-bit path. 32-bit kernelbase leaves
garbage in OUT fields, though, so the host always overwrites an OUT field
and never reads one. `rusty_conhost`'s `console` follows that from PR 2.

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
| 2 | `rusty_conhost` core plus runtime, output only. Covers: `msg`, `console`, output APIs and stubs (Appendix A), the engine loop with `sched`'s fairness tests (§7.2), the two lanes and the §7.4 caps with their stalled-reader and backpressure tests, panic path, `OwnPty`/`OwnPtyMaster`, the rusty_term key parsed but undocumented, the probe example, `cmd /c` fixtures. Interim: input is discarded with one log line, resize only updates the size, Drop closes the server and kills the child after 2 s | 1,150 | 6–8 |
| 3 | Input. Covers: VT/win32-input-mode parser, raw passthrough, reply filter, waiting reads, input event, cooked ReadConsole, Ctrl+C/Break, process groups, GenerateConsoleCtrlEvent, 65001-only code pages. Interactive cmd works. As built: the ctrl worker and its queue land here, since Ctrl+C needs them; cooked editing is typing, Backspace, Escape and Enter, with the cursor keys, history and popups later | 700 | 3–4 |
| 4 | Screen over Grid. Covers: rusty_term_core additions (§4.3, flagged), cursor/attribute/cursor-info APIs, SBI from the Grid, output-mode table, query routing (§5.6), resize (§5.7) with the V3 regression tests. pwsh/PSReadLine and edit.exe start work. As built: with VT processing off and PROCESSED_OUTPUT on, C0 controls other than the five keep their CP437 glyph; §5.4 gained rules 3–5 from the conhost review (legacy wrap, DEL and C1, re-asserting the input modes); DECXCPR (`CSI ? 6 n`) is answered with CPR (V11); the start of a watched sequence at the end of a write is held for the next bytes, and goes out before host VT or at hang-up; on the alternate screen the V3 tests show no divergence, since neither side keeps history there | 700 | 3–4 |
| 5 | rusty_term `OwnBackend` (§4.5), key documented. **Milestone**. As built: `OwnBackend` is the `Backend` and `OwnHandle` the `BackendHandle`, following `WindowsBackend`/`WindowsHandle`; raw mode and the host size stay the ConPTY backend's. The program is split off the `shell` string by `CreateProcessW`'s rules (`backend/cmdline.rs`, §4.5). PR 5 also split the arguments and `winargv` quoted each again, which changed the raw tail cmd.exe reads (`cmd /c echo a"b c"d`); the follow-up adds `Command::raw_args` (a D3 public API change) and passes the tail verbatim. The web bridge stays on ConPTY. The root `Cargo.toml` gains the `rusty_conhost` workspace entry that `rusty_conhost.workspace = true` needs | 300 | 1–2 |
| 6 | FillConsoleOutput, ScrollConsoleScreenBuffer. Clear-Host works. As built (with PR 7, #1330): every cell write paints as conhost under ConPTY does (`VtIo::Writer::WriteInfos`): runs of CUP, SGR and characters between DECSC and DECRC, through the host's Grid, so the terminal and the model get the same bytes. A write queues as one call and paints at most a slice's worth of cells per step, reading what it keeps (attributes under a character fill, a scroll's source) a row at a time, a scroll's rows in memmove order. cmd `cls` and PowerShell's Clear-Host and `[Console]::Clear()` take conhost's GH#3126 shims (#1371): a whole-screen clear by `cmd.exe`, `powershell.exe` or `pwsh.exe` (by CONNECT's ApplicationName) sends `CSI H CSI 2J CSI 3J` instead of cells, so the terminal drops its scrollback as under ConPTY | 450 | 2 |
| 7 | Read/WriteConsoleOutput family, SBIEx, window/buffer size calls. As built (#1333): reads take conhost's `AsCharInfo` view of the Grid; a ReadConsoleOutput reply over 1 MiB is refused with `STATUS_INVALID_PARAMETER` (§7.4). The size calls follow V1, GetLargestConsoleWindowSize reports the window, SetConsoleScreenBufferInfoEx is the V2 no-op | 550 | 2–3 |
| 8 | Lifetime: CTRL_CLOSE (the ctrl worker and its queue are in PR 3), absolute grace deadline, `hang_up`, the §7.3 tests (§5.8). Needs S1. As built: CTRL_CLOSE supersedes queued Ctrl+C/Break and is still delivered after the server closes; a client connecting during the grace gets it; Drop discards output nobody reads | 350 | 1–2 |
| 9 | Code pages via `codec` (§5.9). As built: the two code pages, WriteConsoleA's DBCS lead byte and the slice-wise decode of parked writes (§5.9), V13, `chcp` end to end on Windows | 250 | 1–2 |
| 10 | (reserved for issues found at the milestone) | — | 2 |
| 11 | rusty_mux server: local host choice, `RUSTY_CONSOLE_HOST`, Config field and sites. As built: `ConsoleHost::{ConPty, Own}` in `rusty_mux_server`, a local `Master` enum in the server's Windows pty, and the Windows tests run on both hosts. Beast found Windows PowerShell 5.1 dying at startup on our host, so the host also accepts SetCurrentConsoleFontEx as a no-op, and GetConsoleProcessList (Appendix A) lands here. Per Nano, a clean stop waits for our consoles to shut down (`serve_until`, Ctrl+C/Ctrl+Break, one 6 s server-wide deadline); only an abrupt end remains V15 | 250 | 1–2 |
| 12 | yirp `sessionmgr-pty`: `PtyOptions::console_host`, Send/Sync justification (`sessionmgr-pty/src/lib.rs:99-113, 302-305`) | 150 | 1 |
| 13 | Fuzz crate `rusty_conhost/fuzz` plus a nightly job modelled on `rusty-tls-fuzz` (`ci.yml:1673-1714`). As built: the driver is rusty_conhost's hidden `fuzz` module behind a `fuzz` feature (the core is crate-private), the crate's tests run it on 300 seeded scripts on every PR, the fuzz crate has an empty `[workspace]` table like the other fuzz crates, and the job reuses `tls_fuzz_smoke.sh` | 200 | 1–2 |
| 14 | Windows end-to-end bench and Phase B gate. As built: the `conhost_bench` example times the `gen_workloads.py` corpus through ConPTY and our host, spawn to EOF, alternating hosts, the median of each, and fails past +10% per workload; its client writes through std's stdout (WriteConsoleW) or, under `chcp 65001`, raw. Its first run found writes past the 64 KiB packet cap refused, so they are now streamed (§7.4) | 250 | 1–2 |
| | **Total** | **~5.8k** | **27–39** focused days |
| | Plus S1–S3 | | 1.5 |
| | Plus the D4 Windows run, about 0.5 day for each of the 13 PRs that change code | | 6.5 |
| | **Total with overheads** | | **35–47** |

- **Milestone:** after PR 5. PRs 1–5 are 15–21 focused days, plus S2 (0.5) and five Windows runs (2.5), so **about 18–24 days in**.
- **Compared with the spike report:** its 12–20 days covered only "cmd, pwsh and edit.exe usable". This plan adds lifetime, code pages, the mux and yirp, fuzzing, the bench and test infrastructure.
- **Left out:** the rusty_term ConPTY unification and multiple screen buffers (V6) are deferred.

## 11. Approvals

Every change below is proposed in exact form so it can be co-signed as written (D2) or approved as written (D3). Anything not listed here needs its own approval on the PR that introduces it.

### 11.1 D2: dependency, feature, lockfile, admission and CI changes

No **shipping** third-party dependency is added or upgraded anywhere in the plan. PR 13 does add one external dependency, `libfuzzer-sys = "0.4"`, in an excluded, non-shipping fuzz crate with its own resolved lockfile. Three existing fuzz crates already use it at that version. The approval asked for there covers only that harness and its job, not unrelated lockfile churn. windows-sys features don't appear in `Cargo.lock`, so feature changes don't change any lockfile.

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

**PR 5: rusty_term.** Add three direct, Windows-only dependencies under `[target.'cfg(windows)'.dependencies]`: `platform.workspace = true` (`Command`, `WinSize`, the `PtyMaster` trait), `platform-windows.workspace = true` (to name `WindowsChild`) and `rusty_conhost.workspace = true`. rusty_term has none of them today. There are no re-exports. `Cargo.lock`: rusty_term's `dependencies` list gains exactly three names: `platform`, `platform-windows` and `rusty_conhost`. Cargo records them there even though they're target-specific. There are no additional package changes beyond that named dependency-list change.

**PR 9: platform-windows.** Add `"Win32_Globalization"` to platform-windows' local feature list, and admit `MultiByteToWideChar`, `WideCharToMultiByte` and `IsValidCodePage`. No lockfile change.

**PR 11: rusty_mux_server.** Add `rusty_conhost.workspace = true` under `[target.'cfg(windows)'.dependencies]`. `platform` and `platform-windows` are already direct dependencies there (`rusty_mux/crates/server/Cargo.toml:26, 34`). This is a `rusty_term_core` edge without `l13`, so the mux rule is kept. `Cargo.lock`: one name is added.

**PR 12: yirp `sessionmgr-pty`.** The same edge, `cfg(windows)` only. `platform` and `platform-windows` are already direct dependencies (`sessionmgr-pty/Cargo.toml:29, 35`). `Cargo.lock`: one name is added.

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
pub struct ConsoleServer { /* private */ }          // Send, not Sync: one owner, the engine thread
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

pub struct Waker { /* private */ }                 // Send + Sync
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ready { pub io: Option<IoDescriptor>, pub exited: bool }

pub fn probe() -> Result<()>;
pub fn spawn_attached(cmd: &Command) -> Result<Attached>;
pub fn send_ctrl(pid: u32, event: CtrlEvent) -> Result<()>;

// #1259
pub struct ClientProcess { /* private */ }         // Send + Sync
impl ClientProcess {
    pub fn open(pid: u32, tid: u32) -> Result<ClientProcess>;
    pub fn terminate(&self) -> Result<()>;
}

impl Waker {
    pub fn new() -> Result<Waker>;
    pub fn wake(&self);
}
impl ConsoleServer {
    pub fn submit_read(&mut self, reply: Option<&IoComplete<'_>>) -> Result<()>;
    pub fn wait(&mut self, waker: &Waker, exit: Option<&OwnedWinHandle>, deadline: Option<Instant>) -> Result<Ready>;
    pub fn cancel_read(&mut self);
    pub fn complete_io(&mut self, c: &IoComplete<'_>) -> Result<()>;
    pub fn read_input(&mut self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize>;
    pub fn write_output(&mut self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()>;
    pub fn set_input_available(&mut self, on: bool) -> Result<()>;
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
| `spawn_attached` | Resolves and quotes the program exactly as `WindowsPty::spawn_concrete` does, under the same `SPAWN` lock. It opens the server and its `\Reference`, `\Input` and `\Output`. The child is created with `PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE` plus a `HANDLE_LIST` of only the three std handles, `STARTF_USESTDHANDLES`, and `bInheritHandles = TRUE` (the list limits what is inherited). The call returns once `CreateProcessW` succeeds, so the child's CONNECT is the first packet `wait` reports. `GroupSpec::JoinGroup` gives `InvalidInput`, matching `WindowsPty`. Before `CreateProcessW` it reserves one of 8 process-wide cleanup slots; with every slot holding an unfinished child it fails with `WouldBlock` and creates nothing. A failure before the child exists closes every handle. A failure after it terminates the child and returns setup's error once the exit is confirmed, every handle closed. If the exit isn't confirmed within 5 s, or `TerminateProcess` fails while the child runs, the cleanup error is returned instead, and the reserved slot keeps the child's process handle and console until a worker confirms the exit. A failed wait never releases a slot. Nothing in the call waits longer than that. Draft 3's `size` parameter is dropped: the size belongs to the engine, not the driver. |
| `send_ctrl` | Calls `ConsoleControl(ConsoleEndTask, {pid, hwnd: 0, event, flags})`, where `event` is CTRL_C, CTRL_BREAK or CTRL_CLOSE. `Ok(())` means the call was made, not that the signal arrived: the driver reports success even for a pid that isn't attached to us, and Windows then drops it. `Unsupported` when the export is missing. It may block; rusty_conhost calls it only from the ctrl worker, never from the engine thread or a caller thread (§7.2). |
| `ClientProcess` (#1259) | `open(pid, tid)` is called while the client's CONNECT is unanswered, with the CONNECT thread `tid`. It is `OpenThread(THREAD_QUERY_LIMITED_INFORMATION)` on `tid`, a check that `GetProcessIdOfThread` is `pid` (else `InvalidInput`), then `OpenProcess(PROCESS_TERMINATE \| SYNCHRONIZE)` on `pid` while the thread handle pins the process (§7.3). It fails closed with the OS error otherwise, e.g. a thread that is gone, or `PermissionDenied` for a protected process. `terminate` returns `Ok` for a client that has exited (a zero-timeout `WaitForSingleObject`), and otherwise is `TerminateProcess` with `STATUS_CONTROL_C_EXIT` (`0xC000013A`), the code Windows ends a process with after CTRL_CLOSE. It doesn't wait for the exit, and fails with the OS error. `OpenProcess`, `PROCESS_TERMINATE`, `SYNCHRONIZE`, `TerminateProcess` and `WaitForSingleObject` are already admitted; `OpenThread`, `THREAD_QUERY_LIMITED_INFORMATION` and `GetProcessIdOfThread` are a new D2 admission, proposed on #1286 and co-signed by Nano (2026-10-31), Dottius's co-sign pending. Shape approved by Nano on 2026-10-31 over a Job Object on the child's tree (it would end detached descendants and miss clients outside the tree) and a kill by pid at the deadline (pid reuse); author-reported, as for D1. D3 review is Dottius's |
| `Waker` | An auto-reset event. `wake` is non-blocking and may be called from any thread; several wakes before a `wait` collapse into one. A wake only ends a `wait`; it isn't reported in `Ready`, because the engine reads its lanes on every turn (§7.2). |
| `submit_read` | Starts an overlapped READ_IO, completing `reply` in the same IOCTL when given. Returns at once. A read already in flight gives `WouldBlock` and nothing is sent. The OVERLAPPED block and buffer live inside the `ConsoleServer` at a stable address until that read completes. `BrokenPipe` once the driver has dropped the server. |
| `wait` | Blocks until the in-flight read completes, `waker` fires, `exit` is signalled, or the absolute `deadline` passes; `None` waits without limit, and a deadline already past makes it a non-blocking poll. It then returns a snapshot with no priority between sources: `io` holds the completed read if it has completed, and `exited` is `true` if `exit` is signalled, both checked after the wait ends. Both empty means only the waker or the deadline ended it. The deadline is the caller's `Instant`, so a call cut short by a wake never extends it. A read that completed with an error is returned as that error, and the read is no longer in flight; `exit` stays signalled, so the next call still reports it. Callers stop passing `exit` once they've handled it. |
| `cancel_read` | `CancelIoEx` on the in-flight read, or a no-op when none is. It does not wait. The read's completion, whatever its status (success, cancelled or another error), still arrives through `wait`. No outcome is promised. |
| `complete_io` | Completes one packet without reading the next. It's used for replies that were held back, such as a `ReadConsole` waiting for input. `write_back` longer than the packet's `output_size` gives `InvalidInput`, and nothing is sent. |
| `read_input` / `write_output` | Copy the packet's input payload from `offset`, or write the reply payload at `offset`. They return `InvalidInput` past `input_size` or `output_size`, and `NotFound` if the packet is already complete. `read_input` returns the byte count, which is short only at the end of the payload. |
| `set_input_available` | Sets or resets the manual-reset input-available event that clients wait on. It is idempotent. The engine resets it when the input queue drains (§7.2). |
| `close` / `Drop` | If a read is still in flight, cancels it and waits up to 1 s for its completion. Past that it leaks the read's storage rather than free memory the kernel may still write, logging once. Then it closes the server handle, and the driver disconnects every client, whose pending console calls fail. The engine always drains its read before calling `close` (§7.3), so the bounded wait is a guard. |
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
impl OwnPtyMaster {
    pub fn hang_up(&self);
    pub fn output_ready(&self) -> bool;
    pub fn exit_waiter(&self) -> ExitWaiter;
    pub fn wait_shut_down(&self, timeout: Duration) -> bool;   // #1259
}
pub struct ExitWaiter { /* private */ }               // Send + Sync
impl ExitWaiter { pub fn wait(self); }
```

| Item | Behaviour |
|---|---|
| `spawn_concrete` | Fails with `Unsupported` when `condrv::probe` fails. There's no silent ConPTY fallback (§4.4). `JoinGroup` gives `InvalidInput`. Otherwise it starts the engine thread and the ctrl worker (§7.1) and returns. `size` is the initial window and buffer size; 0 columns or rows gives `InvalidInput`. |
| `read` | Blocks until output is queued, then returns up to `buf.len()` bytes, and sets the control lane's `output_drained` flag when it frees space. It returns `Ok(0)` once the direct child has exited and the queue is drained, which is `PtyMaster::read`'s contract and what `WindowsPtyMaster` does. Concurrent readers are serialized. |
| `write` | Appends VT input bytes from the terminal to the input lane and returns `buf.len()`. It blocks only while 64 KiB of earlier input is still waiting for the engine (§7.4), as a pty write blocks when the child stops reading. It never waits on a client handler. It returns `BrokenPipe` after `hang_up` or once the server is closed. |
| `resize` | Sets the control lane's resize flag; the latest size wins. It never blocks. The engine applies `Grid::resize` (§5.7, including divergence V3), updates the size that `GetConsoleScreenBufferInfo` reports, and queues a `WINDOW_BUFFER_SIZE_EVENT` when the input mode asks for one. 0 columns or rows gives `InvalidInput`. `BrokenPipe` once closed. |
| `hang_up` | Sets the control lane's `hang_up` flag, wakes the engine and returns. It needs no input-lane capacity and calls no OS function except `Waker::wake`. The engine handles it within one turn (§7.2), sends CTRL_CLOSE through the ctrl worker and closes the server at an absolute 2 s deadline (§7.3). It is idempotent. |
| `output_ready` | `true` when a `read` would return without blocking: the queue holds bytes, or the EOF latch is set. |
| `exit_waiter` / `ExitWaiter::wait` | `wait` blocks until the direct child has exited or the console has been torn down, whichever comes first, and returns at once if that already happened. Any number of waiters may exist. |
| `wait_shut_down` (#1259) | Blocks up to `timeout` until the console has closed and every client still attached at its close has been ended, 5 s after CTRL_CLOSE; returns whether it has. Without such a client it returns once the console has closed. A consumer whose process exits right after dropping its last master calls it, or the engine thread doing the kill dies with the process |
| `Drop` | Calls `hang_up` and marks the console detached. The engine tears down and exits on its own (§7.3). Drop never joins a thread or waits on a client. |

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
}
```

| Item | Behaviour |
|---|---|
| `ColorRef` | The colour as currently stored, before reverse video and rendering transforms. `Default` means SGR 39 or 49, or a reset. `Palette(n)` means 30–37, 90–97, 40–47, 100–107 or `38;5;n`/`48;5;n`. `Rgb` means `38;2`/`48;2`. It is **not** necessarily the original value: a `retheme` rewrites matching explicit RGB in the parser and Grid (`core/src/parser.rs:1907-1914`, `grid.rs:3616-3634`), and a Grid restored from RTSN may report former palette cells as `Rgb`, because RTSN doesn't keep palette provenance (`core/src/grid/snapshot.rs`). RTCP does keep it. No serialized state is added for this. |
| `CellColors::flags` | The existing public `ATTR_*` bits. The host reads `ATTR_REVERSE` and `ATTR_UNDERLINE` for `COMMON_LVB_REVERSE_VIDEO` and `COMMON_LVB_UNDERSCORE`. This field is new since draft 3, because `wAttributes` and `ReadConsoleOutput` need those two bits. |
| `pen_colors` | The current SGR pen, the one the next printed character takes. `Pen` itself stays private. |
| `palette` | The live 256-entry palette as `0x00RRGGBB`, including OSC 4, OSC 104 and theme changes. |
| `cell_colors` | The colours and flags of the cell at (`col`, `row`) in the visible screen, with row 0 at the top. A wide character's continuation cell reports its lead cell. `None` when out of range. |

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
| Ctrl delivery deadlocks against the driver | Low | Ctrl on its own worker, a single effect executor, and the §7.2 tests |
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
| 6 | ScrollConsoleScreenBuffer | 0x0200000C |
| 7 | ReadConsoleOutput | 0x02000013 |
| 7 | WriteConsoleOutput / WriteConsoleOutputString / ReadConsoleOutputString | 0x02000011 / 0x02000012 / 0x0200000F |
| 7 | SetConsoleScreenBufferInfoEx (GetConsoleScreenBufferInfoEx is 0x02000007, PR 4) | 0x02000008 |
| 7 | SetConsoleScreenBufferSize / GetLargestConsoleWindowSize / SetConsoleWindowInfo | 0x02000009 / 0x0200000B / 0x0200000E |
| 11 | GetConsoleProcessList | 0x03000029 (draft 6 said 0x0300002A, which is GetConsoleHistory in `conmsgl3.h`) |
| 11 | SetCurrentConsoleFontEx (accepted, ignored; Windows PowerShell 5.1 exits at startup without it) | 0x0300002C |
| #1335 | WriteConsoleInput (W and A, append or prepend; A in the input code page, keeping a DBCS lead byte for the next write) | 0x02000010 |
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
| Emit ordering race | readiness | `handle` appends to the queue itself; since draft 5, on the single engine thread (§7.2) |
| Oversized write parks forever | readiness, design | Soft bound in draft 3; explicit caps and slicing since draft 5 (§7.4) |
| `Pen` not public; `resize` always reflows; palette in parser | codebase, design | §4.3 items 1–4 |
| The ignored ConPTY test is our own bug | codebase | §1.1 and D6 |
| Migration not behaviour-neutral | codebase | Migration dropped (§4.5) |
| Mux and yirp have no config | codebase | §4.4 |
| Feature, admission and fuzz-job approvals missing | codebase | §11.1, in exact form since draft 4 |
| `hang_up` timing | design, codebase | Non-blocking; since draft 5 the deadline belongs to the engine (§7.3) |
| Parser already answers queries | design, codebase | §5.6 |
| win32-input-mode never enabled | design, comprehension | §5.5 startup preamble |
| Over-scoped first landing | zoom-out | Enum, multiple buffers, mux budgets, full code pages and fuzz deferred; milestone after PR 5 |
| Oracle probes arrive last | design, readiness | Probe example from PR 2, grown with each PR (§8.1) |
| No decision summary; divergences scattered; no rollback; no security section | structural, comprehension | Summary table; §5.10; §9.2; §6 |
| Undefined terms, full paths, Rust sigs, hex case, naming | style, comprehension | Glossary, full paths, compiling sigs, uppercase hex, consistent consumer names |

Round 1's disposition table is in `egm-review-1.md`, in this folder. Every round-1 P0 and P1 is either covered by the rows above or by the section it named.

### Round 3: Dottius's review of draft 4 (#1229)

| Item | Resolution in draft 5 |
|---|---|
| 1. Consumer dependency and API surface | rusty_term takes direct, Windows-only `platform`, `platform-windows` and `rusty_conhost` dependencies, with the exact lockfile delta, and no re-exports (§11.1). `OwnPtyMaster::output_ready` and `exit_waiter` are the minimal public methods, and OwnBackend's `exit_token` is an adapter-owned bridge over them (§4.2, §4.5, §11.2.2) |
| 2. Input-event ordering without the Ctrl deadlock | A single engine thread is the only effect executor, so effects apply in the order produced. Ctrl goes to a FIFO worker. Both interleavings and the shutdown races are tested (§7.1–§7.2) |
| 3. Non-blocking shutdown and cancellation ownership | `hang_up` and Drop only enqueue. The engine owns the server and its READ_IO storage, cancels, drains the completion, then calls `close(self)`. The deadline is the engine's. No `Interrupted` promise. Exactly-once EOF (§7.3, §11.2.1) |
| 4. Memory bound | Explicit caps for the client budget, host reserve, emission slice, command queue, waiting calls and parked payload, with admission at each one; stalled-reader tests per producer (§7.4) |
| 5. Colour promise | Narrowed to the stored semantic colour, with the `retheme` and RTSN limits stated (§11.2.3). ANSI↔Win32 is a lossless bit swap, not identical indices (§5.2) |
| 6. `resize_clip` invariants | `resize_clip` is dropped. The host uses the existing `Grid::resize`, whose invariants and tests already cover the alternate screen, saved cursors, tabs, margins and wrap state. This matches conhost's reflow and the terminal's own view (§5.7) |
| Shipping-dependency wording | Done (§11.1) |
| D4 wording | "Successful" run, implementation PRs, a skipped Windows job doesn't count (§8.3) |
| Owner decisions vs verified evidence | Source column in the decision table. The clean-room owner exception is quoted with its date and place (§3.5) |
| §6.8 / §7 references and §10 arithmetic | §6 renumbered, §7 written, references fixed. §10 recomputed: 27–39 focused days, 35–47 with overheads, milestone about 18–24 days |

### Round 4: Dottius's re-review of draft 5 (#1229)

D2 was co-signed for the §11.1 scope at `a6a98ee7`, and the §11.2.3 colour accessors were accepted. Draft 6 answers the three groups that remained:

| Item | Resolution in draft 6 |
|---|---|
| 1. Engine fairness and deadline | `wait` returns a `Ready` snapshot with no priority, and takes an absolute `deadline: Option<Instant>`. Every loop turn checks the deadline, the control lane and child exit, then serves one packet and at most 16 KiB of input, so each is served within one turn. The turn order is a pure `sched` function with always-ready-IO, continuous-input and absolute-deadline tests (§7.2, §11.2.1) |
| 2. Control under backpressure; bounded stores | A fixed-size control lane of coalesced flags (`hang_up`, `detached`, `output_drained`, latest `resize`) is separate from the 64 KiB input lane, is never paused and needs no capacity. The Ctrl queue is capped at 256 entries and coalescing, drops and logs past the cap, and never blocks the engine. Parked bytes are reserved before READ_IO is submitted. The memory figure is scoped to the named buffers, and the screen, client and object tables and title have their own limits. New tests: input and output full together, control behind queued input, Ctrl with a stalled worker, capacity boundaries (§7.1, §7.4) |
| 3. Resize | `Grid::resize` stays, with no new API. The "stay the same" claim is removed, and V3 is restored as a history-dependent content and cursor divergence, using Dottius's trace. S3 measures it and gates Phase B. Shrink/grow and narrow/widen regressions with cursor-positioned and relative writes follow (§5.7, §5.10, §9.1) |
| Stale `cancel` row | Removed; `spawn_attached` now refers to `wait` (§11.2.1) |
| PR 5 lockfile wording | "No additional package changes beyond that named dependency-list change" (§11.1) |
| PR body | Refreshed to draft 6 |

Dottius accepted D3 at the design-contract level on `a3290ce6`, keeping the D2 co-sign in force. The one wording note, that an empty `Ready` means the waker *or* the deadline ended the wait and never by itself means expiry, is applied in §4.1.

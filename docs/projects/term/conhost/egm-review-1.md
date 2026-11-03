# Goldfish Review Report: rusty_conhost design (#1222)

**Target:** `/mnt/project-files/rusty_term/conhost/design.md` (lines 1-171, draft 1, 2026-10-29)
**Evidence read:** `recon-2026-10-29.md`, `spike-1-2026-10-29.md`, `report-2026-10-29.md` (same folder; only these four files were read)
**Codebase checked:** `/home/claude/rusty_mill` (read-only)
**Lenses:** design, comprehension, implementation-readiness, zoom-out, codebase-details, structural, style

> **How this review was run.** No subagent tool was available in this session (only Read/Grep/Glob/Write), so the seven Goldfish reviewers could not be started as separate parallel instances. The synthesizer applied each lens itself, one pass per lens, then deduplicated and ranked the results. Every codebase claim below was checked against the files cited. Treat this as one reviewer covering seven lenses, not seven independent reviewers.

**Severity scale:** P0 = blocks the design (fix before approval) · P1 = major, fix in this revision · P2 = should fix before the PR it affects · P3 = polish.

**Breakdown:** P0: 1 · P1: 12 · P2: 14 · P3: 7

---

## P0: Blocks the design

### P0-1. The "same `platform::pty::Pty` seam" does not exist for rusty_term, and the §4.1 interface can't be built as written
* **Lenses:** codebase-details, design, implementation-readiness
* **Scope:** §1 (line 8), §3.1 table (line 40), §4.1 (lines 79-93), §4.2 (line 97)
* **Problem:**
  1. rusty_term does not use `platform::pty`. `crates/libs/ui/rusty_term/src/backend/mod.rs` defines its own `Backend`/`BackendHandle` traits. `backend/windows.rs` calls ConPTY directly through `windows_sys` (`CreatePseudoConsole`, `ResizePseudoConsole`, ...). `crates/libs/ui/rusty_term/Cargo.toml` has no dependency on `platform` or `platform-windows` at all. `BackendHandle` also needs `try_clone`, `exit_token`, `reap_exit_status`, `output_ready` and `read_into`, and `PtyMaster` has none of these.
  2. "Switching is a one-line choice at each call site" is false for the mux. `crates/apps/rusty_mux/crates/server/src/pty/windows.rs` stores the concrete type in its fields (`Arc<WindowsPtyMaster>` in both `Pty` and `Process`), and it calls `hang_up`, `WindowsChild::kill_single` and `try_wait`. Only `sessionmgr-pty` goes through `impl Pty` (`backend()` at lib.rs:259-261), and it asserts `Send + Sync` on the master (lib.rs:302-305).
  3. `spawn_concrete` returns `WindowsChild`, but `WindowsChild::from_parts` is `pub(crate)` (`platform-windows/src/process.rs:54`). `rusty_conhost`, a separate crate, can't build one. Fixing that needs a new public API in `platform-windows`, and the design doesn't flag it.
* **Fix:**
  - Add a "Consumer integration" subsection with one row per consumer, giving the exact type or trait it will use.
  - **rusty_term:** pick one option and cost it. (a) A `ConhostBackend` adapter that implements `BackendHandle` over `ConhostPtyMaster`, which adds the dependency edges rusty_term → rusty_conhost → platform/platform-windows (needs approval, see P1-9). (b) A behaviour-neutral PR first that moves rusty_term's Windows backend onto `WindowsPty`, so a real shared seam exists.
  - **mux:** make the field type an enum `{ConPty(WindowsPtyMaster), Own(ConhostPtyMaster)}`, or put a small inherent-method trait over the two. This is the second real call site that justifies an abstraction.
  - Put the attached spawn in `platform_windows::condrv` so it returns `(ConsoleServer, WindowsChild)`. `WindowsChild` then stays constructed inside its own crate. List it as new public API.
  - Replace "one-line choice" with the real change at each call site.

---

## P1: Major

### P1-1. No motivation and no alternatives section
* **Lenses:** zoom-out, structural
* **Scope:** §1 (lines 6-14), whole document
* **Problem:** The goal says what will be built (about 4.5k lines on undocumented ConDrv and `ConsoleControl` interfaces, kept up for good) but never why. It doesn't say which ConPTY failures justify the cost. #1220 (bundle conpty.dll/OpenConsole, MIT) appears only as something to close later (§7, line 138). It is never weighed as an alternative. The repo has direct evidence the design could cite: the `#[ignore]` on rusty_term's ConPTY tests ("ConPTY attach broken on Insider 26200.8737", `backend/windows.rs:568-573`), the `STARTF_USESTDHANDLES` leak workaround (`platform-windows/src/sys/pty.rs:330-356`), and version drift in the conhost preamble (recon line 13).
* **Fix:** Add "§1.1 Why" listing the concrete ConPTY defects with links. Add "§1.2 Alternatives considered": (a) status quo, (b) #1220 bundled OpenConsole, (c) own host in-process, (d) own host as a separate exe. For each, give cost, maintenance burden, the risk of Microsoft locking down ConDrv, and what each fixes. Without this the owner can't judge the 12-20 day spend.

### P1-2. The effort estimate doesn't match its evidence
* **Lenses:** codebase-details, implementation-readiness
* **Scope:** §8 (line 154), §3.4 "Spike lines" column (lines 66-75)
* **Problem:**
  - The PR sizes add up to 4,530 lines (400+700+600+80+500+1,000+400+700+150), and the design calls that 12-20 days "from the spike". The report's revised 12-20 days covers 2.2-4.0k *more* lines to make "cmd, pwsh/PSReadLine and edit.exe usable" (report line 61). That smaller scope leaves out multiple clients, extra screen buffers, resize with reflow, wiring three consumers, the probe suite, fuzzing and capture fixtures. The earlier report table puts multiple clients plus screen buffers alone at 3-4 days, resize with reflow at up to 3 days, and hardening at 2-3 days. All of these are in the design's scope.
  - The spike-lines column adds up to 747 (engine) and leaves out the 417 lines of platform plumbing. That is 1,164, not the 1,318 quoted in the header (line 4). `ctrl` = 71 lines appears in none of the evidence files.
* **Fix:** Re-derive the estimate per PR from the report's own table and state the scope each figure covers. Give a range (the report's own method gives roughly 19-32 days for this scope). Source or drop the 71.

### P1-3. The output queue is unbounded, so there's no backpressure, and the mux relies on it
* **Lenses:** design, codebase-details
* **Scope:** §3.3 (line 62)
* **Problem:** "Every screen-changing call appends VT bytes to an output queue ... `PtyMaster::read` drains that queue." Nothing bounds that queue. Under ConPTY a slow reader fills the pipe and conhost blocks the client. Here, `yes`-style output or a stalled consumer grows memory inside the terminal's own process without limit. The mux depends on that backpressure on purpose: `QUEUE = 4` chunks, "the back pressure an unread Linux PTY gives" (`rusty_mux/.../pty/windows.rs:31-33`).
* **Fix:** Bound the queue (for example to 64 KiB of VT). When it's full, don't answer the WriteConsole/RAW_WRITE yet: park it and finish it with `COMPLETE_IO` once `read` drains, the same wait-queue machinery ReadConsole already uses. Add a FakeDriver test for a full queue.

### P1-4. The engine is not kept free of I/O, and the port is never defined
* **Lenses:** design (owner principle: ports and adapters), implementation-readiness
* **Scope:** §3.1 (line 42), §3.3, §3.4 (`ctrl`, `pty`), §6 (line 127)
* **Problem:** §6 depends on a `FakeDriver`, but no driver trait (the port) is defined anywhere: no methods and no home crate. `ctrl` calls `ConsoleControl` and `pty` owns threads and condvars, both inside the same "engine" crate as the pure dispatch logic. So the domain does do I/O, and fake-driver tests can't cover Ctrl handling without real Windows calls.
* **Fix:** Specify a sans-I/O core, roughly `Console::handle(Packet) -> Vec<Effect>`, where `Effect` is one of `Reply`, `Complete(id)`, `EmitVt`, `SendCtrl{pid, event}` or `Log`. Add one narrow port trait (`trait Driver { read_io, complete_io, read_input, write_output }`) plus a `CtrlSink`, with real adapters in `platform_windows::condrv` and fakes in tests. Two implementations (real and fake) meet the "two real call sites" rule. Write the trait signatures in §4.

### P1-5. Possible deadlock: Ctrl delivery under the console mutex
* **Lenses:** design
* **Scope:** §3.3 (lines 55-59), §2 step 5
* **Problem:** The input thread "turns Ctrl+C into ConsoleControl calls", and the driver thread dispatches "under one `Mutex<Console>`". `ConsoleControl(ConsoleEndTask, ...)` makes the system inject a handler thread into the client. That handler commonly calls console APIs (cmd echoes `^C`), and those calls reach the driver thread. If the input thread holds the lock across `ConsoleControl`, or across a blocking `COMPLETE_IO`, the two threads can deadlock. `CTRL_CLOSE_EVENT` waits by design.
* **Fix:** State the rule "no OS call while holding the `Console` lock". The core returns `Effect`s (P1-4), the lock is released, and then effects run. Add a FakeDriver test where a Ctrl handler calls WriteConsole while Ctrl delivery is still in progress.

### P1-6. Console lifetime and `hang_up` differ from today's contract, and CTRL_CLOSE has no PR
* **Lenses:** codebase-details, design, implementation-readiness
* **Scope:** §3.3 item 3 (line 60), §4.1 (line 89), §8
* **Problem:**
  - Today's `spawn_exit_watcher` closes the console when the *direct child process exits* (`sys/pty.rs:624-643`). The design closes it "when the last client disconnects". That is a behaviour change. A `cmd /c start /b` grandchild would hold `read` open after the child exits, which matters to rusty_term's `exit_token` and to mux reaping.
  - `hang_up` is promised to keep "the same contract as `WindowsPtyMaster::hang_up`", which sends every attached program Windows' console-close notification (`pty.rs:147-160`, relied on by the mux at `pty/windows.rs:161-164`). That needs CTRL_CLOSE delivery. The report says CTRL_CLOSE, process groups and Ctrl+Break are "Still open" (report line 53), and none of the nine PRs covers them, yet `ConhostPty` gets wired in at PR 2.
  - Drop behaviour (grace period, then TerminateProcess) isn't specified either.
* **Fix:** Pick one EOF trigger, either "direct child exited and the queue is drained" (matches today) or "last client gone", and record it as a deliberate change if you pick the second. Add a spike item to prove CTRL_CLOSE through `ConsoleControl` and what happens to clients when the server handle closes. Add a PR, before consumers are wired, for `hang_up`, Drop and CTRL_CLOSE/Ctrl+Break/process groups. Consider a Job Object, which this host makes possible (`kill_tree` is `Unsupported` today, `sys/pty.rs:184-196`).

### P1-7. Forwarding terminal queries breaks non-VT-input clients and answers in the wrong coordinates
* **Lenses:** design
* **Scope:** §5.6 (line 120)
* **Problem:** Every DSR/DA query goes to the real terminal, and "its replies come back as input". Two problems:
  - For a client *not* in `ENABLE_VIRTUAL_TERMINAL_INPUT` mode, the reply bytes get parsed into key INPUT_RECORDs, so the client sees typed garbage.
  - A CPR answer from the terminal is in the terminal's viewport coordinates. The client asked about its own buffer, which is the host Grid. Those differ during a resize, or when the terminal is scrolled into scrollback.

  The host Grid is the truth for the client's coordinate space, and the design says so itself (§3.4 `screen`).
* **Fix:** Answer DSR 5/6 (CPR) locally from the host Grid. Forward only what the host can't know (DA1/DA2, XTVERSION, OSC 10/11/4 colour queries), only while the requesting client is in VT-input mode, and match the replies back. Write the routing as a table in §5.

### P1-8. Resize: reflow and repaint fight the terminal's own reflow
* **Lenses:** design
* **Scope:** §4.1 (line 93), §5.1
* **Problem:** "`resize` applies the new size to the Grid, reflows, and repaints." The terminal on top reflows its own Grid too. Its Grid has scrollback and the host's has none (§5.1), so the two reflow results differ: the terminal pulls lines back from scrollback. A host repaint then overwrites the terminal's reflowed view. This is ConPTY's best-known resize bug, and "agree by construction" (§5.3) doesn't hold here.
* **Fix:** Pick one owner of reflow. Suggestion: the host resizes its Grid without reflowing (window-sized buffer, as ConPTY does after 1.22), emits no repaint, and lets clients (PSReadLine) redraw on WINDOW_BUFFER_SIZE_EVENT. Spell out the event and the exact VT emitted. Add a FakeDriver test that resizes during a PSReadLine prompt.

### P1-9. Dependency and lockfile changes need approval, and the design doesn't flag them
* **Lenses:** codebase-details, design (project rules)
* **Scope:** §3.1, §8 PRs 1, 2, 9
* **Problem:**
  - A new workspace member (`rusty_conhost`) adds a `[[package]]` entry to `Cargo.lock`. ADR-0003 (lines 233-235) notes that path members are recorded there.
  - Wiring it into rusty_term adds new edges rusty_term → rusty_conhost → platform/platform-windows. rusty_term depends on neither today.
  - `ConsoleControl` has no windows-sys binding. Getting it means either a hand-written `extern "system"` / `GetProcAddress`, or new windows-sys features. `platform-windows` is the "windows-sys floor (D-1)" with an MSRV of 1.75 (`Cargo.toml:3,6`) and a `track-w` dual-arm convention.

  Project rule: no dependency, lockfile or toolchain change without approval.
* **Fix:** Add "§8.1 Approvals needed": the lockfile change in PR 2, the new edges into rusty_term, the binding approach for the undocumented user32 export (recommend a runtime `GetProcAddress` that fails cleanly to `Unsupported`), whether `condrv` needs a Track W arm, and confirmation that the code compiles at MSRV 1.75 in `platform-windows`.

### P1-10. The rusty_term_core API additions aim at the wrong types
* **Lenses:** codebase-details
* **Scope:** §4.3 (lines 101-109)
* **Problem:**
  1. `Grid::pen()`: the pen isn't on `Grid`. It's a private field of `AnsiParser` (`core/src/parser.rs:69`). `Pen` is public with public `fg_index`/`bg_index` (`core/src/cell.rs:133-148`).
  2. `Cell::palette_index() -> Option<u8>`: the public `Cell` (`cell.rs:260-280`) has no index fields. The provenance lives in the `pub(crate)` `CellStyle` (`style.rs:29-42`). One `Option<u8>` can't hold both foreground and background. The tag also covers `38;5;n` up to 255, so 4-bit read-back needs a check for 0..=15. `Cell` is a public struct with public fields, so adding fields breaks struct-literal callers.
  3. `AnsiParser::set_wrap(bool)` isn't needed: DECAWM is already handled (`parser.rs:1271-1272`, `g.autowrap`). The host can just emit `CSI ?7l` / `CSI ?7h`, which also keeps the "everything goes in as VT" rule.

  The "the PR checks that before adding anything" hedge can be settled now.
* **Fix:** Replace §4.3 with: (1) `AnsiParser::pen(&self) -> &Pen`; (2) `Grid::cell_palette(col, row) -> (Option<u8>, Option<u8>)` (or a `CellColours` struct) that reads the existing `fg_index`/`bg_index` tags without changing `Cell`; (3) drop `set_wrap` and use DECAWM. Also say how the host Grid is built with zero scrollback.

### P1-11. Windows tests don't run on PRs, and agents can't queue
* **Lenses:** codebase-details, implementation-readiness
* **Scope:** §6 (line 126), §8 (line 142), §9 row 1
* **Problem:** "Each PR is small, green on Windows CI", and the Windows integration tests are the main mitigation for "undocumented ConDrv ... changes". But in `.github/workflows/ci.yml` (lines 41-43), "a PR runs Linux only, plus Windows clippy; the queue runs Windows". Windows *tests* first run in the merge queue, and agents never queue (AGENTS.md). So every PR in the sequence reaches Dottius with its Windows behaviour untested. Also unverified: whether `ConsoleControl` and ConDrv server creation work in a GitHub-hosted runner session, and how runner builds (Server 2022/2025) compare with Beast's 26300.
* **Fix:** State how Windows tests run before review. Options: ask Dottius for a manual `workflow_dispatch` run, or a label-triggered Windows test job. Either is a CI-control change and needs approval. As the first task of PR 1, run a one-off runner probe of ConDrv plus `ConsoleControl`, and record the runner build as a third data point.

### P1-12. Public API changes are not all flagged
* **Lenses:** design (owner principle), codebase-details
* **Scope:** §3.1, §4.1, §4.2, §4.3
* **Problem:** Only §4.3 is marked "flagged on the PR". Also public and unflagged:
  - `platform_windows::condrv`. Note `pub mod sys` (`platform-windows/src/lib.rs:77`), so `sys::condrv` raw functions become public too.
  - The new `WindowsChild` construction path (P0-1).
  - The whole `rusty_conhost` crate surface.
  - The `windows.console_host` config key, a user-facing config change in rusty_term, the mux and yirp.
* **Fix:** Add a "Public API changes" table: item, crate, PR, breaking or not. Make `sys::condrv` `pub(crate)` and expose only the safe wrapper.

---

## P2: Should fix before the affected PR

### P2-1. Spawn details from the spike and the repo are missing from §2
* **Lenses:** codebase-details, implementation-readiness
* **Scope:** §2 steps 3-4 (lines 20-25)
* **Problem:** Left out:
  - `SET_SERVER_INFORMATION` registering the input-available event (spike line 8, recon line 20). Programs that wait on the stdin handle need it, and it must be set and reset as the queue fills and empties.
  - The duplicate of `\Output` used for stderr (spike line 9).
  - What the repo already learned the hard way: `STARTF_USESTDHANDLES` must be set or the parent's redirected handles leak into the child (`sys/pty.rs:330-356`), and console creation is serialised (`pty.rs:32-36`, `SPAWN` mutex).
  - Using a `HANDLE_LIST` also needs `bInheritHandles = TRUE`, unlike today's spawn.
* **Fix:** Add these to §2 and to PR 1's scope, and reuse `sys::nt`'s existing `NtCreateFile` + `RootDirectory` open (`sys/nt.rs:86-126`) instead of a new one.

### P2-2. Code-page rules contradict each other and break real users
* **Lenses:** design, style
* **Scope:** §1 Non-goals (line 12), §5.7 (line 121)
* **Problem:**
  - "Only 65001 is accepted" is followed straight away by "437 is accepted".
  - Treating 437 as 65001 changes byte meaning for `WriteConsoleA`/`ReadFile` on consoles with CP437 output (box-drawing bytes in old tools).
  - It isn't said what `GetConsoleCP` returns at startup (ConPTY returns the OEM CP, e.g. 437).
  - Rejecting everything else breaks `chcp 932/936/949/850` for CJK and European users, a visible regression from ConPTY that the probe parity gate would catch.
* **Fix:** Settle the rule: report 65001 at startup (a documented divergence), accept SetConsoleCP/OutputCP for any installed code page, and convert with `MultiByteToWideChar` in the platform layer. If that's out of scope, say which locales regress and make it a Phase-B blocker.

### P2-3. Panic containment is overclaimed
* **Lenses:** design, implementation-readiness
* **Scope:** §3.2 (line 50), §9 row 4 (line 163)
* **Problem:** "Caught at the thread boundary" and "covered by FakeDriver fuzz". The only fuzz target in §6 is `msg` decode, not the engine. A panic while holding `Mutex<Console>` poisons it for the other threads, and the design doesn't say what they do then. Pending client calls must be completed (failed) or the clients hang. The owner principle forbids `unwrap` in library code; the repo's own pattern is `unwrap_or_else(PoisonError::into_inner)` (`pty.rs:67`).
* **Fix:** Specify the failure path: `catch_unwind` around dispatch, mark the console dead, fail every pending IO with `STATUS_UNSUCCESSFUL`, close the queue so `read` returns `Ok(0)`, and handle poison in the other threads. Either add an engine-level fuzz target that feeds packet sequences into `Console::handle`, or reword the risk row.

### P2-4. In-process trade-off not weighed for the mux server
* **Lenses:** zoom-out, design
* **Scope:** §3.2 (lines 44-51), Non-goals line 14
* **Problem:** The in-process argument is made for rusty_term. The mux server hosts many consoles in one long-lived process. Any attached program can now send arbitrary packets into that process. Before, the worst it could do was crash its own conhost.exe. A host deadlock, memory blow-up (P1-3) or CPU spin affects every block. "Not a security boundary" covers privilege, not availability.
* **Fix:** Add per-packet limits (cap `InputSize`/`OutputSize` before allocating, a per-console memory budget) and a watchdog. Note the separate exe as the trigger option for the mux specifically. Answer open question 3 with this in mind.

### P2-5. Fuzz target placed in a lower-layer harness
* **Lenses:** codebase-details, structural
* **Scope:** §6 Fuzz (line 129)
* **Problem:** "In the existing rustils fuzz crate." `crates/platform/rustils/fuzz` is a standalone platform-family harness that depends only on `platform` and `platform-windows` (`fuzz/Cargo.toml:14-15`). `msg` lives in the libs crate `rusty_conhost`, so this points platform tooling up at libs. The fuzz workflow runs only the `winargv_roundtrip` target (`rustils/.github/workflows/fuzz.yml`).
* **Fix:** Use `crates/libs/ui/rusty_conhost/fuzz/` (precedent: `crates/libs/ui/rusty_term/fuzz`), add it to the root `exclude` list, and add a nightly job step.

### P2-6. The property test can't fail
* **Lenses:** design, implementation-readiness
* **Scope:** §6 (line 127)
* **Problem:** "Grid state equals feeding the emitted VT into a fresh rusty_term_core". §3.4 `screen` builds the Grid *by* feeding that same VT, so the two sides always match. It can't catch a wrong translation of a Win32 call into VT.
* **Fix:** Use independent oracles: expected `CHAR_INFO`/cursor state recorded from real conhost for each probe, and golden VT for each API. Keep the round-trip check only as a drift guard for code paths that update the Grid directly, if there are any.

### P2-7. FakeDriver captures are underspecified, and some don't exist
* **Lenses:** implementation-readiness, codebase-details
* **Scope:** §6 (line 127)
* **Problem:**
  - The "edit.exe captures" can only cover startup, because edit.exe never ran usefully (report line 15).
  - The captures live on Beast, outside the repo.
  - Interactive replay depends on timing (pending reads, input arrival).
  - CONNECT payloads (1,340 bytes, spike line 27) carry app path, title and current directory, not just history.
* **Fix:** Define a text capture format with explicit input events and expected replies, a sanitiser that scrubs CONNECT fields, paths and env, where the fixtures live, and a size cap. Say that edit.exe fixtures are produced in PR 7.

### P2-8. Probe clients as `tests/probes/*.rs` won't work as described
* **Lenses:** implementation-readiness
* **Scope:** §6 Probe clients (line 128)
* **Problem:** Files under `tests/` build as libtest harness binaries, not standalone console programs that a host can spawn. The sentence also mixes up "against stock conhost" and "compares ConPTY and rusty".
* **Fix:** Make the probes `[[bin]]`/`examples` of a dev-only crate (another workspace member, so add it to P1-9's approvals), or one probe binary with subcommands. State the oracle: the same probe under ConPTY (inbox conhost) on the same runner, with captured `CHAR_INFO`/mode/cursor results compared field by field.

### P2-9. PR sequence gaps and sizing
* **Lenses:** structural, implementation-readiness
* **Scope:** §8 (lines 140-154)
* **Problem:**
  - No PR covers CTRL_CLOSE/Ctrl+Break/process groups (P1-6), `GenerateConsoleCtrlEvent` (P2-10), the fuzz target, the probe suite or capture fixtures.
  - PR 6 at about 1,000 lines isn't "small".
  - PR 2 exposes `ConhostPty` behind a user-selectable key before input works (PR 3).
  - rusty_term wiring is split between PR 2 and PR 9.
  - Change fragments aren't mentioned.
* **Fix:** Split PR 6 into Fill/Scroll and Read/WriteConsoleOutput. Add a "lifetime and signals" PR. Keep the config key undocumented or debug-only until PR 3. Do all rusty_term wiring in one PR after P0-1 is settled. Note one `changes/` fragment per PR.

### P2-10. APIs that are needed but not mentioned
* **Lenses:** design, zoom-out
* **Scope:** §5, §3.4
* **Problem:**
  - `GetConsoleWindow`: ConPTY gives clients a hidden pseudo-window HWND, and programs that call `ShowWindow`/`GetConsoleWindow` misbehave on NULL.
  - `GenerateConsoleCtrlEvent`: a client-initiated Ctrl event to a process group, used by Python, Node and build tools.
  - `GetConsoleScreenBufferInfoEx`/`SetConsoleScreenBufferInfoEx` colour table: what palette is reported, and does Set emit OSC 4?
  - `GetNumberOfConsoleInputEvents`/`FlushConsoleInputBuffer`.
* **Fix:** Add a "required API set" table (ApiNumber → PR → behaviour) built from the census plus these. For `GetConsoleWindow`, decide NULL (documented divergence) or a message-only window.

### P2-11. Non-VT output mode needs more than ESC handling
* **Lenses:** design
* **Scope:** §5.5 (line 119)
* **Problem:**
  - With `ENABLE_VIRTUAL_TERMINAL_PROCESSING` off, `ENABLE_PROCESSED_OUTPUT` still governs BS/TAB/BEL/CR/LF, and with it off those bytes print as glyphs.
  - "ESC is printed as a visible character (it goes through the parser as a literal)" is impossible as written. Under the "everything goes in as VT" rule the host must *translate* ESC (conhost shows U+2190 ←) before the parser sees it, or the terminal will act on it.
  - `ENABLE_WRAP_AT_EOL_OUTPUT` / DISABLE_NEWLINE_AUTO_RETURN aren't covered.
* **Fix:** Add a small table of the output-mode bits and the VT emitted for each byte class, with a unit test for each row.

### P2-12. Ctrl+C under VT input and win32-input-mode
* **Lenses:** design
* **Scope:** §3.3 item 2 (lines 56-59)
* **Problem:** It doesn't say how Ctrl+C is spotted when it arrives as `0x03`, as a win32-input-mode `CSI ..._` record (rusty_term already sends those, `backend/windows.rs:604-611`), or while a cooked read is pending (the read must end, CtrlWakeupMask aside). Under `ENABLE_VIRTUAL_TERMINAL_INPUT` with processed input, conhost still raises the event.
* **Fix:** Add a decision table: input encoding × mode bits → (event sent, record queued, pending read result). Cover it with FakeDriver tests.

### P2-13. Minimum Windows version and runtime detection
* **Lenses:** zoom-out, implementation-readiness
* **Scope:** §7 Phase B (line 137), §9 row 1
* **Problem:** "One older 10 or 11 build" isn't a support policy. When did `\Reference` and `PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE` arrive? What happens on an older build where opening `\Device\ConDrv\Server` fails or `ConsoleControl` is missing?
* **Fix:** State the minimum build, probe for it at startup, and return a clear `Unsupported` error. That is consistent with "no silent fallback" (§4.2) and with user-visible reporting.

### P2-14. Open question 1 understates the cost of a theme
* **Lenses:** codebase-details, structural
* **Scope:** §10 Q1 (line 168)
* **Problem:** A new `libs/term/` theme isn't just a path choice. `check_workspace_layers.py` hard-codes `LIBS_THEMES` (line 18). ADR-0003's table would need an amendment. Reaching three crates would mean moving `rusty_term` (and `rusty_ansi`?) out of `libs/ui/`, with CI path updates.
* **Fix:** Say so, recommend `crates/libs/ui/rusty_conhost` with `[package.metadata.rusty_mill] layer = "libs"`, and drop the question unless the owner wants the move.

---

## P3: Polish

### P3-1. Claims not backed by the evidence
* **Lenses:** codebase-details, comprehension
* **Scope:** §2 (line 28)
* **Problem:** "A PSReadLine session uses 13 APIs over 241 messages" appears in none of the three evidence files. "Interactive cmd works fully" overstates the report's "echo, dir, exit, titles" (report line 13).
* **Fix:** Cite the source (the runs3 log on Beast) or remove the figure, and say "works for echo/dir/exit/title".

### P3-2. Broken cross-references
* **Lenses:** structural, comprehension
* **Scope:** §3.2 (line 51), §3.3 (line 62)
* **Problem:** "Recon §7d" doesn't exist in the recon file in this folder, which has no numbered sections; it points to Beast's private copy.
* **Fix:** Point to "recon, 'ConPTY pipe protocol'" and "recon, 'Flushing'", or copy the material into the folder's recon file.

### P3-3. Undefined terms
* **Lenses:** comprehension
* **Scope:** §4.3 (line 109), §2, §3.4, §6
* **Problem:** RTCP/RTSN, Beast, CtrlWakeupMask, InitialNumBytes, "parity gate", rt-caps.sh and "DISCONNECT semantics" are used without definition.
* **Fix:** Add a short glossary, or define each on first use. Say what RTCP/RTSN are and why they'd bump.

### P3-4. No data-flow picture
* **Lenses:** comprehension, structural
* **Scope:** §3
* **Problem:** A reader has to put together terminal ↔ `ConhostPtyMaster` ↔ queues ↔ engine ↔ ConDrv ↔ client from three subsections.
* **Fix:** Add one ASCII diagram showing threads, locks and queues.

### P3-5. Style and consistency
* **Lenses:** style
* **Scope:** §4.2, §3.4, §8, §2
* **Problem:**
  - The config value `"rusty"` is vague next to `"conpty"`.
  - The "Spike lines" column uses "—" for both "new" and "unknown".
  - PR sizes give "~400 lines" once and bare "(~700)" afterwards.
  - Function codes are listed without the numbers the recon gives.
  - "LangId" vs "LangID".
* **Fix:** Use `"conpty" | "own"` (or `"rusty_conhost"`), write "new" in the column, keep units consistent, and add codes (1 CONNECT ... 8 RAW_FLUSH).

### P3-6. Where the MIT notice goes
* **Lenses:** design (project rules)
* **Scope:** §3.4 `msg` (line 68), §9 Licence (line 164)
* **Problem:** "No other source is copied" is a strong claim when the cooked-read and VT-input behaviour will likely follow microsoft/terminal closely. Where the notice lives isn't stated.
* **Fix:** Name the notice location (file header in `msg` plus a `THIRD_PARTY_NOTICES` entry with the commit `09beddc6`) and the rule: any ported logic, not just layouts, carries the notice.

### P3-7. The report's placement recommendation is overridden without saying so
* **Lenses:** structural
* **Scope:** §3.1
* **Problem:** The report recommended "`rusty_conhost` under platform-windows" (report line 50). The design splits it across platform and libs, which is better, but doesn't note the change.
* **Fix:** One sentence noting that the design deliberately departs from the report's recommendation, and why.

---

## Verified as accurate (no action)
- `rusty_wire` exists as an in-repo foundation crate, already used by rusty_term_core for bounds-checked reads.
- ADR-0003: libs may depend on platform; a theme needs three or more crates.
- `platform-windows` already has `ffi`, `sys::handle`, `sys::nt` (`NtCreateFile` with `RootDirectory`), and `winargv` (re-exported crate).
- `WindowsPtyMaster::hang_up` and `spawn_exit_watcher` exist as described. Only the trigger differs (P1-6).
- `fg_index`/`bg_index` provenance exists (on `Pen` and `CellStyle`, not `Cell`). See P1-10.
- Windows CI exists (`windows-latest` in the queue and sweeps; Windows clippy on PRs). See P1-11.
- rusty_term's config supports sectioned keys (`apply(cfg, section, key, ...)`, `config.rs:697`), so `windows.console_host` fits.
- `crates/libs/ui/rusty_term/extra/rt-caps.sh` exists.

> **ARCHIVED.** Closed out: row 1 shipped (PR #266); row 2 turned out to be
> a false lead, not a real gap — retracted rather than merged (see below).
> Kept for the historical record, not as a live backlog.
>
> - **`CreatePseudoConsole`'s `PSEUDOCONSOLE_INHERIT_CURSOR`** — closed
>   #263, merged in
>   [PR #266](https://github.com/baileyrd/rusty_win32/pull/266) as
>   `conpty::create_with_flags`.
> - **`UpdateProcThreadAttribute`'s `PROC_THREAD_ATTRIBUTE_REPLACE_VALUE`**
>   — implemented in
>   [PR #267](https://github.com/baileyrd/rusty_win32/pull/267), but real
>   `windows-latest` CI failed reproducibly (`Win32Error(31)`,
>   `ERROR_GEN_FAILURE`) on two separate pushes of identical code.
>   Microsoft's official `UpdateProcThreadAttribute` documentation states
>   `dwFlags` "is reserved and must be zero"; `PROC_THREAD_ATTRIBUTE_REPLACE_VALUE`
>   does not appear anywhere in that current, official parameter
>   documentation — only mingw-w64's `processthreadsapi.h` declares the
>   constant, and it does not correspond to a functional capability on
>   real Windows. PR #267 was closed without merging; issue #264 was
>   closed as not-planned. `AttributeList::update`'s existing hardcoded
>   `dwFlags = 0` was already correct.
>
> Left as a caution for future parity-loop passes: a header-declared
> constant is a *candidate*, not proof of a working capability — real
> Windows behavior (or, short of that, current official documentation) is
> the actual source of truth, and mingw-w64's headers can carry stale or
> otherwise non-functional declarations.

# Gap analysis: rusty_win32 `conpty` vs. the real Win32 ConPTY API surface

Reference surface: `wincon.h`'s ConPTY declarations (`CreatePseudoConsole`/
`ResizePseudoConsole`/`ClosePseudoConsole`, the `PSEUDOCONSOLE_INHERIT_CURSOR`
flag) and `processthreadsapi.h`'s `PROC_THREAD_ATTRIBUTE_LIST` mechanism
(`InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`/
`DeleteProcThreadAttributeList`, including the `dwFlags` parameter on
`UpdateProcThreadAttribute`'s own call site inside `conpty::AttributeList`),
both read directly from mingw-w64's headers (installed on this machine for
this pass — same reliable proxy for `windows-sys`/`windows` used by
`docs/archive/gap-analysis.md`'s earlier, broader sweep) rather than assumed
from memory.

Scope: this run is narrowly `conpty` (`src/conpty.rs` +
`process::spawn_suspended_with_pseudoconsole`) against the real Win32 ConPTY
surface specifically — not the wider `PROC_THREAD_ATTRIBUTE_*` mechanism used
by unrelated `CreateProcessW` features (job lists, mitigation policies,
group affinity, etc., dozens of which exist in `winbase.h`). Those stay
explicitly out of scope for this run: `AttributeList` was added *for* ConPTY
and is reused generically, but auditing its fitness for every other Win32
feature that happens to use the same mechanism is a separate, much larger
initiative this run does not attempt.

Already wrapped and verified matching the real declared signatures: all
three `CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole`
functions (parameter order confirmed against the header, not just assumed),
the full `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`/
`DeleteProcThreadAttributeList` group, `HPCON` (`VOID*` in the header,
`*mut c_void` here), `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`'s computed value,
and `STARTUPINFOEXW`'s layout (`size_of`/`align_of`/`lpAttributeList` offset
all compiler-probe-verified in `process.rs` already). No signature or layout
mismatches found — the existing implementation is faithful to the header.

The header's ConPTY declaration block (`wincon.h` lines 405-416) is short and
exhaustive: three functions plus exactly one flag constant
(`PSEUDOCONSOLE_INHERIT_CURSOR`). Two real, narrow, additive gaps survive —
both single hardcoded-to-zero flag parameters, not missing functions.

| Symbol | Category | Source | Platforms | Reference | Breaking? | Est. size | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `CreatePseudoConsole`'s `dwFlags` (`PSEUDOCONSOLE_INHERIT_CURSOR`) | fn (existing) | spec | windows | `wincon.h` line 409/413 | no | S | `conpty::create` hardcodes `flags: 0` in its `CreatePseudoConsole` call, so `PSEUDOCONSOLE_INHERIT_CURSOR` (the one documented flag — inherit the parent terminal's cursor position rather than starting at 0,0) can never be requested. Fix as a new `create_with_flags` (or equivalent) alongside the existing `create`, not a signature change to it. |
| `UpdateProcThreadAttribute`'s `dwFlags` (`PROC_THREAD_ATTRIBUTE_REPLACE_VALUE`) | fn (existing) | spec | windows | `processthreadsapi.h` line 217/224 | no | S | `conpty::AttributeList::update` hardcodes `flags: 0` in its `UpdateProcThreadAttribute` call. Windows documents `PROC_THREAD_ATTRIBUTE_REPLACE_VALUE` as required to successfully call `update` a second time on the same attribute ID (e.g. rebinding a different `HPCON` into an already-initialized list) — without it that second call fails. Fix as a new `update_with_flags` (or equivalent) alongside the existing `update`, not a signature change to it. |

No other gaps found: `ResizePseudoConsole`/`ClosePseudoConsole` take no
flags at all in the real header, and every `PROC_THREAD_ATTRIBUTE_*`
constant besides `PSEUDOCONSOLE` itself belongs to an unrelated
`CreateProcessW` feature, out of scope per this run's own framing above.

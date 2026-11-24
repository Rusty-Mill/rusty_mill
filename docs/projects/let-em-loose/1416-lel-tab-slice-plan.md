# LEL tab in rusty_term: slice plan (card #1416)

Status: plan, no code. Design is merged (gui-design D53-D58, `assets/html/lel.html`, PR #1510). This note only orders the build.

## Scope

Read-only Let Em Loose tab: intent brief, claims map, decision inbox, per-claim spend. Inbox actions (Approve, Reject, Choose) go through the same host `bbp human` commands as the #595 inbox (`rusty_mill_board/src/bbp_cli.rs`, D35): displayed revision as `--rev` and `--fence`, `dry_run` first, a stale revision is refused and re-read, never retried. No push in v0. No BBP or orch change (D55).

## Data source: Mill Board HTTP API (Nano, 2026-11-24)

`bbp` and the BBP log live on the orch host; rusty_term runs on the desktop as a mux client. Nano chose (a) on a decision card in the project thread, 2026-11-24T08:07:40Z. Options were:

- a) Mill Board HTTP API: inbox reuses `inbox_list`/`inbox_act`; claims, intent and spend need new read endpoints (public API change). Recommended: smallest, one writer path, same fence.
- b) `bbp` over SSH or a mux block: waits on Windows `--ssh` (C79) and credentials.
- c) A new mux `Features` call proxying read-only BBP views and the actions: larger protocol change.

The inbox reuses `inbox_list`/`inbox_act` with their revision fence and `dry_run`. Claims, intent and spend need new read endpoints on Mill Board wrapping `bbp`: a public API change that gets its own PR and owner gate before slices 2, 5 and 6 can show live data. Slice 1 does not depend on it.

## Types

rusty_term does not depend on `rusty_mill_board` (sqlite, oauth, serve; ADR-0003 keeps `bbp` a process boundary). Default: the view model and parsing live in rusty_term, with golden JSON fixtures copied from `bbp inbox --json`, plus a drift test parsing the same fixtures in both crates. Extracting a shared crate is a new crate (owner gate), not done here.

## Slices (one small PR each, stacked)

1. **View model.** Pure parsing and fixtures; happy path, empty and error states; no UI.
2. **Claims map and read-only intent panel** (D54-D56). Map from `claim_card`/`claims_delta`; groups derived from `domain` and `deps` (Q40 interim); undo log shows nothing or a static "not built" (Q41 interim). Stage strip derived read-only (D55).
3. **Inbox list and top-strip chip** (D53, D22 pattern). Live wiring needs the data-source answer.
4. **Approve, Reject, Choose** through host commands, Approve with the D58 confirm (Cancel focus first, Reject, "Accept claim"; no execution wording). "Try both builds": candidates side by side, nothing launched (Q43 interim). Needs the data-source answer.
5. **Spend meter**: per-claim `spent` and `budget` only; lens list from orch config (Q42 interim).
6. **Windows run**: paste-ready PowerShell prompt in `/mnt/project-files/let-em-loose/`; the GUI is judged on Windows (D63).

## Gates

- D25: Nano approves this plan before slice 1 starts.
- Open questions Q40-Q43 are non-blocking; each has an interim choice above.
- Q9 (exact LEL fields behind map and inbox): bind to D57 names, confirm with `let-em-loose-expert` when slice 2 starts.

## Experts

GUI visual design: `rusty-term-design-expert` (pack only in the cloud). GUI code: `rusty-term-expert`. BBP and inbox semantics: `let-em-loose-expert`.

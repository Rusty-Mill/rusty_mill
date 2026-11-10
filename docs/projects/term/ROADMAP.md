# Rusty Term roadmap (with rusty_mux)

Status: **owner answered all 13 questions in the project thread on 2026-11-04** (main 83e70a1c);
they count once Nano confirms them on the PR that adds this file. Items still each need their
own go to start; no item goes to the Mill Board's
Ready column before that. Mux phases, exit criteria and capability rows stay in
[../mux/ROADMAP.md](../mux/ROADMAP.md) and [../mux/CAPABILITIES.md](../mux/CAPABILITIES.md);
this file orders the work across both and does not copy them.

**Live view:** the Mill Board milestones for projects `rusty_term` and `rusty_mux` (one card per
item, each with a prompt and its dependencies). This file is the narrative; when the two differ,
the board is current and this file is refreshed (last refresh 2026-11-10, main 0fbc83ee).

## Where we are

- **Engine and front-end.** The old term roadmap is complete (last item #1150). Scrollback
  blocks (#6) and glyph caches (#12) were skipped by the owner; scroll step 2 skipped, step 3
  deferred ([HANDOFF.md](HANDOFF.md) §2).
- **Windows.** Our own console host `rusty_conhost` is the default since #1403; ConPTY is
  rollback only, with no auto-fallback. The VT bulk path (#1433) made the host ~3x faster.
  GUI repaint is paced (#1423).
- **GUI.** Chrome style guide and theme-derived search colours landed (#1460). Clipboard keys
  follow Windows Terminal habits (#1402, #1411). First-run setup (O11, #1471); chrome size fixed
  apart from the terminal font (#1482); redesign slices 1-3 (#1523, #1528, #1536).
- **Mux.** Phases 0, 1, 2 and 2b done. Phase 3: transport (C13) done; identity partly done
  (principals, login mode without PAM). Phase 3b done (C37–C41, C50–C52, C71, C73 #1481, C74
  #1490 and #1516). Arc 3 (C45, C70, C69, C36a) done; agent hosting C46 and C77 done.
- **GUI as a mux client (arc 2).** S1-S5 merged (#1532 to #1554), Take over (#1577), and
  `rusty_term --gui` now runs `rusty-mux-gui` (#1569). Deleting the local-mode code (R2) waits
  for its own go.

## Next, in proposed order

Each item is small and needs no new crate or dependency. Order avoids file conflicts.

| # | Item | Why now | Gate |
|---|---|---|---|
| 1 | ~~Finish O11 PR 1~~ **Done** (#1471) | Owner said yes; touches `gui/settings.rs` | none beyond review |
| 2 | ~~Decouple terminal `font_size` from the chrome `ui_font`~~ **Done** (#1482) | Last step of the fixed chrome scale ([STYLEGUIDE.md](STYLEGUIDE.md) §Size); after O11 to avoid settings conflicts | none |
| 3 | ~~Docs hygiene: HANDOFF §4 queueing rule and §3.1 floors are stale (pack Q1, Q2)~~ **Done** (card #784, 2026-11-10; pack D154, D16) | One PR, no code; stops agents reading old rules | none |
| 4 | Conhost next ceiling: profile `Screen::client`/forward (~53%) and the mirror-grid parse (~32%) on Beast, measurements first (card #785) | Host is now the Windows path; numbers decide whether a fix is worth it | none for the profile; the fix is a separate go |
| 5 | ~~Skill text read from the installed binary (Orca O3, Rex D1); new row C73~~ **Done** (#1481) | XS; both peers do it; installed skill stays matched to the binary | Nano's go on the row; reconfirm the agent paths (K4) |
| 6 | ~~Agent hook plus OSC 7501 status precedence (Orca O2)~~ **Done** (C74: #1490, #1516) | Richer agent status, OSC as the fallback on hosts without hooks | Nano's go; a new `Features` bit if peers see new events |
| 7 | Snapshot encode reliably at or below libghostty (ROADMAP Phase 0's open ≤ 11.2 ms target, #939): profile, then fix (card #786) | Owner wants it tried, not waved off (question 13) | none; measured on a fixed host |
| 8 | Config key for mode 2027 grapheme clustering, off by default (question 13 B; card #787) | Owner said yes | none |
| 9 | Move rusty_term's ConPTY rollback backend onto platform-windows's wrapper (question 13 C; card #788) | Owner said yes | none |

## After that: the larger arcs

Each arc needs the owner's go before it starts, and its own plan.

1. **Mux Phase 3 remainder: sharing and identity.** *Not started; needs Nano's go (first card 507).* C14 live sharing, C46 block ownership,
   C35 several servers, C25/C26 rest, C27 Tailscale identity, C47–C49 remote setup.
   Gates: C33's force-disconnect must be restricted first (card 507); C27 needs the
   `rusty_tail` move (ADR-0003); PAM links libpam (native dependency); C47/C48 write to
   remote hosts and `~/.ssh/config`; TLS (card 506) touches `rusty_tls`.
2. **One GUI for terminal and mux.** *Under way: S1-S5 and Take over merged; R1 merged (#1569), R2 waits for its go.* The terminal-mux design (session switcher, split-tree
   thumbnails, server-owned layout, per-block status) is a requirement. The GUI becomes a mux
   client only, keeping its local mode until C69 and C45 land (question 1). Both have landed: `rusty_term --gui` now runs `rusty-mux-gui` (retirement step R1, handoff D146), and deleting the local-mode code (R2) waits for its own go. This is where mux Phase 4's native client (C16) and C21,
   C42, C53–C55, C61–C64 land. Visual design goes to the design pack once it is on main.
3. **Sessions that survive restarts.** *Done except C36b clean stop (card 1010, owner call) and C69 (d) utmp (deferred, Q29): C45 #1488, C70 #1493, C69 (a) #1555, (b) #1558, (c) #1563, C36a #1570.* C45 (state survives a server restart) plus C70
   (scrollback spooled to disk), pulled forward from Phase 5 (question 2); it gates arc 2.
   Then C36 upgrade without losing sessions and C69 auto-start, with Orca's endpoint-ownership
   rules (O6) first.
4. **Let Em Loose on the mux (proposal).** *Agent hosting rows C75-C79 designed (AGENT-HOSTING.md); C46 #1556 and C77 #1560/#1561 done; C75, C76, C78, C79 each need Nano's go.* No link exists today between `rusty_orch`/`rusty_bbp`
   and rusty_mux. Agents are launched by the `bbp mod` harness launcher, which keeps their whole
   lifecycle (process group, slot, grace, kill, reap); `bbp runner` only runs test profiles. So the
   mux does not fork agents: that would be a new executor and needs Nano's go. Proposed v0 is a
   watch-only agent pane: a block that tails the harness's turn log and polls `bbp card`, with
   status from BBP's log, never written back into it. Inside the BBP sandbox Unix sockets are
   blocked, so any status a confined agent sends the mux comes as OSC 7501 on the pty, not an O2
   hook calling the mux socket. Open calls (agent blocks without the mux socket in their
   environment; read-only attach) are in the convergence brief and need Nano.
5. **Mux Phase 4 rest and Phase 5.** Web client, WebSocket API, device pairing, config
   migration, parking, encryption at rest, audit, plugins (C66 is a new crate). Order set when
   arc 2 is under way.

Windows debts to clear along the way: ConPTY spike 1 and the Windows fidelity corpus (moot
since #1403?), the two-account check, and Windows runs of C50 and C71.

## Questions for Nano

Each has a recommendation. Answer on the PR so the reviewer can accept it.

1. **GUI architecture.** *Answered (Nano, project thread, 2026-11-04 12:06Z: "Concur"):* the
   GUI ends as a mux client only, as Rex and Orca do (REX-APP rows 38 and A9), reached through a
   standalone-plus-attach stage: rusty_term keeps its local mode until C69 (auto-start, idle exit)
   and C45 (sessions survive a server restart) land, then retires it. Nano to restate on this PR.
2. **Restore after restart.** *Answered (Nano, project thread, 2026-11-04 12:06Z: "Q2 yes"):*
   C45 + C70 move forward from Phase 5. It forces question 3. Nano to restate on this PR.
3. **Scrollback over 10k rows.** *Answered (Nano, project thread, 2026-11-04 12:07Z: "Q3
   concur"):* keep `SCROLLBACK_MAX = 10_000` rows in memory; older rows spill to disk with C70.
   The disk limit is set in C70's plan. Nano to restate on this PR.
4. **Binary-loaded skill (O3).** *Answered (Nano, project thread, 2026-11-04 12:11Z: "Concur"):*
   becomes C73 and the next mux step. The installed `SKILL.md` is a stub that reads the text from
   `rusty-mux-ctl`; if ctl is not on PATH it says so, never a stale copy. The agent paths (K4) are
   reconfirmed on the C73 PR. Nano to restate on this PR.
5. **Agent hook plus OSC 7501 status (O2).** *Answered (Nano, project thread, 2026-11-04 12:12Z:
   "Concur"):* adopt, built after C73. Hooks carry prompt, tool, reply and subagent state; OSC 7501
   fills in where hooks are silent or missing. A new `Features` bit if peers see new events. Nano to
   restate on this PR.
6. **tmux shim (O4).** *Answered (Nano, project thread, 2026-11-04 12:16Z: "No for now"):* the
   CHARTER non-goal stands. Agents split panes through the skill's `split-block`; Claude Code's
   built-in team panes stay in-process. Revisit if team panes become a daily need. Nano to restate
   on this PR.
7. **Agent orchestration (O7/O8).** *Answered (Nano, project thread, 2026-11-04 12:18Z: "I concur,
   HOWEVER take a look at Rusty_orch and Rusty_bbp"):* rusty_mux stays a substrate, plus O7
   `create-session --worktree`. Orchestration already has a home: `rusty_orch` plans goals into
   task cards and `rusty_bbp` is the blackboard with store-enforced roles, budgets and human gates
   (Let Em Loose). So O8 is not built in the mux; the mux's job is to be a good place for those
   agents to run (see arc 4). Nano to restate on this PR.
8. **FreeBSD** as a server target? *Answered (Nano, project thread, 2026-11-04 12:22Z: "Q8 not
   now"):* not now. Nano to restate on this PR.
9. **Version mismatch (with C36).** *Answered (Nano, project thread, 2026-11-04 12:23Z: "Agree but
   we should also notify the user of that because they may be expecting capabilities the server
   can't support"):* negotiate down through `Features`, refuse only below a floor, and tell the
   user when the other side lacks features their client has (ctl, attach client and GUI show which
   are missing and that the server is older). Nano to restate on this PR.
10. **Card 283 (ConPTY convergence gate).** *Answered (Nano, project thread, 2026-11-04 12:24Z:
    "Concur"):* the gate is moot; the ConPTY review a year after Phase B stays. The card itself
    stays open (Backlog) for its real remainder: Phase 1's acceptance suite on Windows on our host,
    lifting the Linux-only cfg on the 4 MiB reattach test if it passes. Nano to restate on this PR.
11. **C33 force-disconnect.** *Answered (Nano, project thread, 2026-11-04 12:25Z: "Concur"):*
    restricted to the session's owner before shared mode, as the first step of arc 1 (card 507).
    Nano to restate on this PR.
12. **Clean room across the merged project.** *Answered (Nano, project thread, 2026-11-04 12:30Z:
    "Yes"):* the rusty_conhost ruling covers rusty_mux and rusty_term_core too: licence
    compatibility is the test, and MIT code is copied with its notice (as `rusty_conhost`'s
    THIRD_PARTY_NOTICES.md does for microsoft/terminal). No copied code was found outside
    rusty_conhost (grep, 2026-11-04). Nano to restate on this PR.
13. **Smaller carry-overs.** *Answered (Nano, project thread, 2026-11-04 12:32Z: "A. We should
    try, not just hand wave / B. Yes / C. Yes"):*
    - Snapshot encode reliably at or below libghostty's time is a real target: next item 7.
    - A config key for mode 2027 grapheme clustering, off by default.
    - rusty_term's ConPTY rollback backend moves onto platform-windows's wrapper.
    Nano to restate on this PR.

## How this file is kept

Update it in the PR that finishes or reorders an item. Capability rows and phase exits stay
in the mux docs; owner decisions stay in the DECISIONS files they belong to.

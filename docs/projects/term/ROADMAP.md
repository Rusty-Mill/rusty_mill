# Rusty Term roadmap (with rusty_mux)

Status: **draft for the owner's review** (2026-11-04, main 83e70a1c). Nothing here is
approved until Nano says so on the PR that adds it; no item goes to the Mill Board's
Ready column before that. Mux phases, exit criteria and capability rows stay in
[../mux/ROADMAP.md](../mux/ROADMAP.md) and [../mux/CAPABILITIES.md](../mux/CAPABILITIES.md);
this file orders the work across both and does not copy them.

## Where we are

- **Engine and front-end.** The old term roadmap is complete (last item #1150). Scrollback
  blocks (#6) and glyph caches (#12) were skipped by the owner; scroll step 2 skipped, step 3
  deferred ([HANDOFF.md](HANDOFF.md) §2).
- **Windows.** Our own console host `rusty_conhost` is the default since #1403; ConPTY is
  rollback only, with no auto-fallback. The VT bulk path (#1433) made the host ~3x faster.
  GUI repaint is paced (#1423).
- **GUI.** Chrome style guide and theme-derived search colours landed (#1460). Clipboard keys
  follow Windows Terminal habits (#1402, #1411).
- **Mux.** Phases 0, 1, 2 and 2b done. Phase 3: transport (C13) done; identity partly done
  (principals, login mode without PAM). Phase 3b done (C37–C41, C50–C52, C71). Phases 4 and 5
  not started.
- **In flight.** First-run setup page (Orca gap O11), first PR: shell, theme, clipboard and
  notification steps; the agents step comes later.

## Next, in proposed order

Each item is small and needs no new crate or dependency. Order avoids file conflicts.

| # | Item | Why now | Gate |
|---|---|---|---|
| 1 | Finish O11 PR 1 (in flight) | Owner said yes; touches `gui/settings.rs` | none beyond review |
| 2 | Decouple terminal `font_size` from the chrome `ui_font` | Last step of the fixed chrome scale ([STYLEGUIDE.md](STYLEGUIDE.md) §Size); after O11 to avoid settings conflicts | none |
| 3 | Docs hygiene: HANDOFF §4 queueing rule and §3.1 floors are stale (pack Q1, Q2) | One PR, no code; stops agents reading old rules | none |
| 4 | Conhost next ceiling: profile `Screen::client`/forward (~53%) and the mirror-grid parse (~32%) on Beast, measurements first | Host is now the Windows path; numbers decide whether a fix is worth it | none for the profile; the fix is a separate go |
| 5 | Skill text read from the installed binary (Orca O3, Rex D1); new row C73 | XS; both peers do it; installed skill stays matched to the binary | Nano's go on the row; reconfirm the agent paths (K4) |
| 6 | Agent hook plus OSC 7501 status precedence (Orca O2) | Richer agent status, OSC as the fallback on hosts without hooks | Nano's go; a new `Features` bit if peers see new events |

## After that: the larger arcs

Each arc needs the owner's go before it starts, and its own plan.

1. **Mux Phase 3 remainder: sharing and identity.** C14 live sharing, C46 block ownership,
   C35 several servers, C25/C26 rest, C27 Tailscale identity, C47–C49 remote setup.
   Gates: C33's force-disconnect must be restricted first (card 507); C27 needs the
   `rusty_tail` move (ADR-0003); PAM links libpam (native dependency); C47/C48 write to
   remote hosts and `~/.ssh/config`; TLS (card 506) touches `rusty_tls`.
2. **One GUI for terminal and mux.** The terminal-mux design (session switcher, split-tree
   thumbnails, server-owned layout, per-block status) is a requirement. The GUI becomes a mux
   client only, keeping its local mode until C69 and C45 land (question 1). This is where mux Phase 4's native client (C16) and C21,
   C42, C53–C55, C61–C64 land. Visual design goes to the design pack once it is on main.
3. **Sessions that survive restarts.** C45 (state survives a server restart) plus C70
   (scrollback spooled to disk), pulled forward from Phase 5 (question 2); it gates arc 2.
   Then C36 upgrade without losing sessions and C69 auto-start, with Orca's endpoint-ownership
   rules (O6) first.
4. **Mux Phase 4 rest and Phase 5.** Web client, WebSocket API, device pairing, config
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
6. **tmux shim (O4).** tmux compatibility is a CHARTER non-goal. Reverse it for a narrow shim
   covering only the commands Claude Code agent teams use? *Recommend no for now;* revisit if
   agent teams become a daily need.
7. **Agent orchestration (O7/O8).** Should rusty_mux grow ask/reply, gates and worktrees, or
   stay a substrate other tools run on? *Recommend substrate, plus O7 worktree-per-session.*
8. **FreeBSD** as a server target? *Recommend not now.*
9. **Version mismatch (with C36).** Refuse, warn, or negotiate down when client and server
   differ? *Recommend negotiate via `Features`, refuse only below a floor.*
10. **Card 283 (ConPTY convergence gate).** Moot since our host is the default? *Recommend
    close it, with the ConPTY review a year after Phase B kept.*
11. **C33 force-disconnect.** Restrict to the session's owner before shared mode?
    *Recommend yes, as the first step of arc 1.*
12. **Clean room across the merged project.** Does "contamination is not a concern, licence
    compatibility matters" (rusty_conhost) extend to rusty_mux and the core? *Recommend yes,
    with the MIT notice rule.*
13. **Smaller carry-overs:** snapshot encode speed against libghostty a hard target (no,
    recommended); a config key for mode 2027 clustering (yes, off by default); after Phase B,
    move rusty_term's ConPTY rollback backend onto platform-windows's wrapper (yes, recommended).

## How this file is kept

Update it in the PR that finishes or reorders an item. Capability rows and phase exits stay
in the mux docs; owner decisions stay in the DECISIONS files they belong to.

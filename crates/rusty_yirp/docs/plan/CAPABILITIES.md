# Xirp — Observed Capabilities List

Sources (both public, black-box — no app.asar/source code used, per the
clean-room boundary in `SCOPE.md`):
1. Third-party review video ["Spotify Made an Agent Harness... I Don't Hate
   It"](https://www.youtube.com/watch?v=TK1LvtX-Bqs) by Better Stack
   (2026-08-15) — hands-on walkthrough, independent commentary.
2. Official Spotify R&D announcement video ["Introducing Xirp: The Agentic
   Development Environment Built by
   Spotify"](https://www.youtube.com/watch?v=pnNkTtFdBYQ) (2026-08-10) —
   65s marketing spot, publicly published on YouTube. Descriptions below
   paraphrase its claims rather than quote its ad copy verbatim.
3. Third-party hands-on video ["Spotify's Xirp Runs 50 AI Agents At
   Once"](https://www.youtube.com/watch?v=8WppW2ImqMw) by Creator Magic
   (2026-08-12) — longest and most detailed walkthrough of the three,
   including install/onboarding, keyboard shortcuts, and an explicit
   caveats/limitations list near the end. Independent (not Spotify), but the
   creator is clearly enthusiastic/promotional in tone — weigh accordingly.
4. Third-party analysis video ["Spotify Just Built an AI Coding Tool... And
   It's Weird!"](https://www.youtube.com/watch?v=xldHyMBB17U) by Panda
   Making Money (2026-08-14). **Important distinction from sources 1 and
   3: this is desk research, not a hands-on demo** — no screen capture of
   actually using Xirp appears anywhere in the transcript; it synthesizes
   Spotify's own launch post/engineering blog and public community reaction
   (e.g. replies on the launch announcement), with an explicitly skeptical
   editorial angle. Treat its architecture claims as restatements of
   Spotify's marketing/engineering-blog framing, not independent
   verification, and treat its one claim that conflicts with hands-on
   sources (see Account/access below) as likely an error.

5. Podcast segment from ["#63 Spotify Xirp, Webflow massive announcement,
   Manus Splits From Meta, Grok Bot, Meta Muse
   Glimmer"](https://www.youtube.com/watch?v=nnBvcTVGNZg) by Cmd+AI
   (2026-08-15) — a ~90-minute multi-topic show; Xirp is one segment
   (roughly the transcript's "Spotify" mentions cluster). **Also not
   hands-on** — one host explicitly says "I haven't used it" — the segment
   is two people reading Spotify's own Xirp/Portal landing-page copy aloud
   and reacting to it live, plus recounting Twitter/X replies to the
   launch. Lowest signal-to-noise of the five sources; kept mainly for one
   claim that conflicts with a hands-on source (see Limitations below) and
   an editorial comparison worth noting for competitive framing.

Cross-referenced with `backstage.spotify.com/docs/xirp` per earlier research.

Caveat: this is one reviewer's ~8-minute pass, not exhaustive. Some "doesn't
seem to" items may be missing features, may exist but undiscovered by the
reviewer, or may require setup the reviewer didn't have (e.g. skills symlink
issue he flags as possibly self-inflicted).

## Account / access

- Requires sign-up with a **Spotify Technology account** (distinct from a
  consumer Spotify account) — gated access, not anonymous/local-only.
- Per source 3: sign-up specifically **requires a work email domain** —
  Gmail, Yahoo, and Outlook addresses are rejected at sign-up.
- **Source conflict**: source 4 (desk-research, not hands-on — see source
  list above) describes the login as "an actual Spotify account, the same
  kind you would use to stream music." This directly contradicts sources 1
  and 3, both of which demonstrate on-screen that it is a distinct *Spotify
  Technology account* requiring a work email, explicitly not the consumer
  music account. We treat the hands-on sources as authoritative here and
  source 4's claim as a research error — but it's flagged rather than
  silently dropped, since if this project ever needs to double-check
  against Spotify's own materials directly, this is exactly the kind of
  claim worth re-verifying rather than trusting any single video.

## Onboarding / setup flow (source 3)

- Install is a standard macOS drag-to-Applications `.dmg` flow (matches our
  own container inspection).
- First-run wizard: pick a coding agent to configure (Claude Code, Codex;
  Gemini CLI offered as an install-on-demand option), toggle **session
  hooks** on/off, choose a default pane layout, review keyboard shortcuts,
  finish.
- On first session creation it verifies **prerequisite CLIs are
  installed**: GitHub CLI (`gh`) and `tmux` — consistent with source 1's
  inference that `tmux` underlies the terminal/pane implementation; `gh`
  suggests PR/GitHub-integration features exist somewhere (not otherwise
  observed by any of the three sources).

## Top-level structure

- **Home page**: single prompt input box + a project selector; submitting
  creates a session against the chosen project.
- **All Projects page**: list of configured projects, each showing active
  session count / total session count; can create a new project from here.
- **Project page** (per project), with sub-tabs:
  - **Overview** — prompt box (same as home), list of active sessions,
    worktree info, an activity log.
  - **Git** — same git diff view as the session-level Git panel, scoped to
    the project.
  - **Files** — built-in file editor over the project's files.
  - **Skills** — meant to surface agent "skills" (reviewer's own skills
    weren't picked up — possibly a symlink-related bug, unconfirmed).
  - **Rules** — read/edit view of picked-up rule files (reviewer saw his
    global and project-level `CLAUDE.md` here); files are editable in-app.

## Session management

- **Multi-agent support**: manages Claude Code, Codex, and Gemini sessions
  from one place. Each session runs the actual underlying CLI inside a real
  terminal (Xirp does not reimplement the agents — "just managing them").
- **Model/effort selection at session creation** (source 3): choosing an
  agent isn't just CLI-vs-CLI — you also pick the underlying model and
  reasoning effort (e.g. "Claude Fable 5, Extra High effort" vs. switching
  down to Opus; Codex shown running a "GPT 5.6" default). Also offers a
  **plan-mode** toggle (at least for Claude Code) and a **run-in-background**
  option at session start, plus a folder-trust confirmation prompt.
- **Three distinct session-start actions** in the sidebar (source 3), not
  one: **New Session** (agent runs in the current project directory —
  multiple such sessions in the same project appear to share working-copy
  state/context with each other), **New Work Tree Session** (explicitly
  spins up an isolated git worktree so the agent can't collide with other
  agents), and **New Terminal** (plain shell, no agent). Worktree isolation
  therefore reads as an *explicit per-session choice*, not a default applied
  to every session — a meaningfully different model than source 1 implied.
- **Sessions persist independently of the Xirp app process**: quitting Xirp
  entirely does not kill running agent sessions — they keep running under
  `tmux` in the background; relaunching Xirp reattaches to whatever is still
  active. (Consistent with tmux being a hard dependency, not just used for
  one feature.)
- **Sessions view**: per-session terminal, sidebar listing every session
  across all projects, single-session-at-a-time by default.
- **Context/token usage display**: each session shows live context-window
  usage against the model's total (e.g. "X of 1,000,000 tokens"), including
  spend visibility even on a subscription plan rather than metered API
  billing.
- **Grid / multi-pane layout**: toggle (keyboard shortcut **Cmd+G**, source
  3) to view multiple sessions simultaneously side by side; "add all"
  populates the grid from active sessions; grid size/columns/rows are
  configurable; pane splits are drag-resizable. Source 3 reports spinning up
  sessions in the grid with no hard-coded cap encountered ("until Xirp
  complains") — see the "50 agents" scale claim under Limitations below.
- **Non-agent panes**: a grid pane can also be a plain terminal (not tied to
  an agent session) — e.g. work manually in one pane while watching agents in
  others.
- **Command palette** (Cmd+K, source 3): run actions like starting a new
  session or renaming a session without leaving the keyboard.
- **Session switcher** (Cmd+Shift+K, source 3): quick-switch between active
  running sessions.
- **Session states** observed: active/running, idle, "just finished,"
  "needs your action" (i.e. agent is blocked waiting on user input — this
  state is what session hooks are designed to surface, see Extensibility
  below).
- **Fork session**: clones an entire session including full context/chat
  history into a new, independent session — lets you branch a conversation
  and send it in a different direction without losing the original thread.
- **Switch agent mid-session**: change which underlying CLI (e.g. Claude Code
  → Codex) a session uses *without* losing chat history/context — the
  transcript is migrated to the new agent and work continues seamlessly.
  Demonstrated independently by both source 1 and source 3 (source 3 has the
  new agent summarize prior work correctly as a live proof). Flagged by both
  reviewers as a standout, uncommon feature.
- **Dependent sessions**: create a child session tied to a parent session's
  worktree; the child can be configured to wait for the parent session to
  finish before starting (with a "start now" override to skip the wait) —
  effectively a simple sequential task chain.
- **Dependent terminal sessions**: a plain terminal pane linked to an agent
  session's worktree (same worktree, grouped together in the sidebar) — for
  manual work alongside an agent in the same workspace.

## Git / review tooling (per session)

- **Git sidebar panel**: lists changed files for the session's worktree.
- **Diff view**: click a changed file to see its diff inline.
- UI is full-screen when open (reviewer's complaint — no persistent sidebar
  diff view like some competitors offer).

## Other in-session tooling

- **Embedded browser**: opens a browser pane scoped to the session, to view
  what a web-facing agent task is doing live.
  - Reviewer could not find a way for the *agent* to control this browser or
    read its console logs, nor any way to reach devtools — appears to be
    view-only, human-facing (unconfirmed whether this is a real limitation or
    just not surfaced in the UI he used).
  - Source 3 shows it **auto-detecting a local dev server** the agent just
    started (e.g. `localhost:8000`) and offering to attach/open it as a
    numbered tab tied to that session — not just a manually-navigated
    browser pane.

## Cost / model routing (source 4, restating Spotify's own engineering post — not independently verified hands-on by any of the three other sources)

- Beyond just picking a model/effort at session start (confirmed hands-on by
  source 3), Spotify's own framing describes **mid-task model/vendor
  routing**: a running session can be switched to "whichever option offers
  the best combination of price and performance," including routing to
  **open-source models Spotify hosts itself** rather than paying per-token
  to a frontier provider. Positioned explicitly as avoiding lock-in to any
  single vendor's pricing. No source in this doc has shown this in the UI —
  it may be the same mid-session agent-switch feature (sources 1 & 3)
  described from a cost-optimization angle, or a distinct routing feature.
  Worth verifying rather than assuming it's a separate mechanism.

## Extensibility: session hooks (source 3)

- Xirp can install **agent CLI hook scripts** (shown editing Claude Code's
  settings JSON, both a project-local script and a global hook so it applies
  across all projects) that fire on session lifecycle events: **agent
  needs your action** (blocked/stuck), **session complete**, and **a
  sub-agent/orchestrated task finishes**.
- These hooks are what let the in-app status badges (idle / needs action /
  finished) update. Enabling them is offered as an opt-in step during
  onboarding.
- Xirp itself has **no built-in outbound notification channel** (no mobile
  app, no push API) — the reviewer had to point the hook at an external
  webhook (he used Zapier) to get a phone notification when a session
  finishes or gets stuck. This is a capability *gap* Xirp explicitly exposes
  a hook for, but doesn't solve itself — worth noting as "extensible via
  hooks, not self-sufficient for remote alerting."
- **Open in terminal**: opens the project/worktree in the user's system
  terminal.
- **Open in editor**: opens the project/worktree in the user's configured
  code editor.

## Underlying implementation signals (still black-box, but inferable from behavior)

- Worktree-per-session model — confirmed by dependent-session language
  ("new child worktree from that worktree").
- `tmux` appears to be a runtime dependency (reviewer needed it installed for
  a terminal-related feature to work) — suggested as a likely implementation
  detail of the terminal/pane management, not confirmed by source.

## Org-context integration (adjacent product, not in Xirp itself)

- Spotify's **Portal** (their Backstage-descendant) is a separate product;
  Xirp integrates with it to pull organizational/project context into
  sessions for large orgs running many concurrent projects. Reviewer frames
  this as the actual strategic reason Spotify built Xirp — a lead-gen /
  stickiness surface for Portal — rather than Xirp being valuable
  standalone.
- Per Spotify's own announcement (source 2), the Portal integration is
  bidirectional: a session **pulls in** component architecture, dependency
  graph, and ownership data from Portal before the agent starts writing
  code, and **writes back** session transcripts/metadata to Portal when the
  session ends — intended so the next person or agent (teammate or
  otherwise) picks up with that accumulated context instead of starting
  from zero. This is Spotify's stated core value proposition for pairing
  Xirp with Portal ("a true agent command center"), not independently
  confirmed by the third-party reviewer, who doesn't appear to have Portal
  connected in his walkthrough.
- Source 4 (restating Spotify's engineering blog, not independently
  verified) adds more detail on what "pulls in" covers: component
  architecture, **dependency graphs**, **ownership topology** (which teams
  own which parts of the codebase), and **prior architectural decisions**.
  It also frames Portal as a **shared marketplace for skills, rules,
  plugins, and agent-tool configs** across an org — teams can discover and
  build on each other's configs instead of each team quietly duplicating
  the same setup. This is the same underlying claim as source 2's "shared
  skills/rules/MCP configs," just with more specificity about the
  mechanism (a catalog/marketplace inside Portal, not something inside
  Xirp itself).

## Team/shared-config capabilities (source 2 only — not covered in the review video)

- **Shared skills, rules, and MCP server configs across a team**, rather
  than each engineer maintaining their own local setup. This is a distinct
  claim from the per-project Skills/Rules tabs the reviewer saw (which
  appeared to be local/per-project, not team-shared) — worth treating as a
  separate capability to verify rather than assuming they're the same
  mechanism.
- Framed explicitly as supporting **"dozens of agents across the same
  codebase in parallel without conflicts"** — i.e. worktree isolation is
  positioned as a scaling property (many engineers, many agents, one repo),
  not just a single-user convenience.
- Scale claim (marketing, not a feature per se): 1,300+ Spotify engineers,
  36,000+ sessions run through Xirp to date (repeated independently by
  source 3 with the same numbers, framed as "before the beta opened").
- Source 3's title claims **"up to 50 agent sessions at once"** — not
  independently verified against an actual hard limit in-app; the reviewer
  says he could keep spinning sessions up "until Xirp complains" but doesn't
  show hitting that ceiling on camera. Treat 50 as a marketed/tested figure,
  not a confirmed hard cap.

## Explicitly NOT done by Xirp (per reviewers, i.e. non-capabilities/limitations)

- Does not replace or reimplement Claude Code / Codex / Gemini — pure
  session/UI wrapper around the real CLIs.
- No visible support for tagging/referencing project files or skills from
  the project-level prompt box (source 1 found this "pretty useless" as-is).
- No auto-update visible in-product beyond the generic electron-updater
  config already known from container inspection.
- **Single machine, no remote access** (source 3, stated as an explicit
  caveat): no server deployment mode, no way to SSH into a session, nothing
  reaches your phone natively — "it stops at the glass." This is why the
  reviewer had to build his own Zapier bridge off the session hooks.
  - **Unresolved conflict**: source 5, reading directly from Xirp's own
    landing-page copy on air, quotes the phrase **"run sessions locally or
    remotely."** That's Spotify's own marketing language, not the
    podcast hosts' claim, and it sits awkwardly next to source 3's
    hands-on finding of no server/SSH/remote capability at all. Possible
    explanations: "remotely" could mean a remote *git repo/codebase*
    rather than a remote *machine running Xirp*, it could be a
    Portal-gated capability neither hands-on source had access to, or it
    could be aspirational copy ahead of the actual beta feature set. Not
    resolved by any source here — flag as a specific thing to verify
    directly (e.g. against `xirp.spotify.com` copy or Spotify's
    engineering post) before assuming either reading.
- **No data redaction**: per source 3, session transcripts are *not*
  scrubbed of credentials, personal data, or other sensitive info before
  being uploaded (presumably to Spotify's backend / Portal). Stated as a
  caution, not confirmed against any published data-handling doc — treat as
  an important open question if this project ever needs a real privacy
  posture, not as verified fact.
- **Closed source, no license, no public repo** — explicitly contrasted by
  source 3 against Spotify open-sourcing Backstage in 2020; Xirp is not
  getting the same treatment.
- **macOS only.**
- Sign-up gated to a Spotify Technology account with a **work email**
  (Gmail/Yahoo/Outlook rejected) — both source 1 and source 3 hit this.
- Per source 3, **most of the differentiated value is gated behind Spotify
  Portal**, which individual users/most companies won't have access to —
  echoes source 1's read that Xirp's real purpose is a Portal on-ramp.

## Credibility notes on the scale/adoption numbers (source 4)

- Spotify published **two different adoption figures on the same launch
  day** through two different official channels: Spotify Engineering's own
  account stated "1,300+ engineers," while the companion engineering blog
  post (by the SVP of Platform) stated "thousands of engineers... across
  more than 36,000 sessions." Source 4 points out these are never
  reconciled or explained (no methodology, no time window given) and treats
  both as directional marketing rather than audited fact. Doesn't mean
  Xirp isn't used internally — it clearly is — just that the specific scale
  claims shouldn't be taken as precise.
- Source 4 also notes **no independent, outside-Spotify benchmarks exist**
  for reliability, overhead, or how well context actually survives
  mid-session agent switches — everything published so far is Spotify's own
  framing of its own product. Relevant caveat for how much weight to put on
  any of the four sources' more impressive-sounding claims in this doc.

## Competitive landscape

Source 4 named several existing tools solving an overlapping problem;
**Solo (soloterm.com) and its comparison pages were directly fetched and
verified** (2026-08-16, not from a video transcript — see
`SCOPE.md`'s competitive-landscape section for the full writeup and the
resulting scope decision). Summary:

- **Solo** by Aaron Francis — a Tauri-based (native, no Electron) desktop
  process dashboard for running CLI coding agents (Claude Code, Codex,
  Gemini CLI, Amp, OpenCode, custom) alongside a dev stack. **Runs on macOS
  and Windows today** (Linux "coming soon"). Explicitly **not** a
  git-worktree orchestrator — coordinates parallel agents in a *shared*
  workspace via lease-based locks, shared key-value state, scratchpads,
  todos, and 40+ MCP tools, rather than isolating them into separate
  worktrees. Proprietary: free tier capped at 4 projects/20 processes, Pro
  is $99/yr.
- **Conductor** (Melty Labs, YC S24) — the actual worktree-isolation
  competitor: runs Claude Code/Codex in parallel, each in its own isolated
  git worktree, with built-in diff review and PR creation. Free, but
  **macOS Apple Silicon only** (Intel "in development"), and limited to
  Claude Code/Codex (no Gemini CLI, no arbitrary CLI tools).
- **OpenCode Desktop** — tabbed multi-session layout for managing several
  agent sessions side by side (per source 4; not independently verified).
- **Claude Code itself** — reportedly gained its own cross-session
  messaging / agent-listing features (per source 4; not independently
  verified), which would somewhat overlap our MVP's `list`/`attach`
  commands.

**The gap this surfaces**: nothing currently combines *git-worktree
isolation* with *Windows support*. Xirp has both but is Spotify-gated/
macOS-only; Conductor has worktree isolation but is Apple-Silicon-only;
Solo runs on Windows but deliberately doesn't do worktree isolation. See
`SCOPE.md` for how this changes the project's stated differentiator.

Source 5 adds one more framing worth keeping, purely as a design-space
comparison rather than a Xirp capability: they describe the Portal/"living
documentation" concept as functionally similar to **Obsidian** (a
local-markdown-file knowledge graph) with an agent placed in front of it —
i.e. a shared, interlinked set of plain files an agent reads before acting,
versus a hosted proprietary knowledge service. That's a genuinely
lightweight, non-proprietary pattern (markdown files + agent context
injection) worth considering if this project ever wants a Portal-like
"shared org context" feature without building or depending on anything
resembling Portal itself.

## Reviewer's overall take (context, not a capability)

Competent, no bugs besides the skills tab; feature set is not perceived as
differentiated enough to justify the account gate + closed-source nature for
an individual developer. Reviewer's personal daily driver is an open-source
alternative (open-source Codex-like tool, referred to in the video as "T3
Code"). Reviewer's take: Xirp's real value is as an on-ramp to Spotify's
Portal product for large orgs, not as a standalone tool for individuals.

---

## Mapping to `SCOPE.md`

Everything above under "Session management" and "Git/review tooling" maps
directly onto the MVP slice already scoped (`new`/`list`/`attach`/`close`,
worktree-per-session). Not yet in scope, worth a deliberate decision:

- Fork session (clone session + context) — real scope add, not free.
  **Shipped in Phase 6** (Claude Code first, then Codex and Gemini CLI) —
  see `../phase-6-report.md`.
- Switch agent mid-session while preserving context — the standout feature;
  depends on being able to translate one CLI's conversation format into
  another's resumable state, which is nontrivial per agent pair.
  **Shipped in Phase 7**, across all three CLIs, via a rendered-transcript
  handoff rather than a native per-CLI resume — see `../phase-7-report.md`.
- Dependent/chained sessions — sequencing on top of the worktree model.
  **Shipped in Phase 5** — see `../phase-5-report.md`.
- Grid/multi-pane layout — already flagged as v1.1 in `SCOPE.md`.
  **Shipped**: `sessionmgr-tui`'s grid (Phase 4) and `sessionmgr-desktop`'s
  own port of the same layout math (Phase 8).
- Embedded browser, in-app file editor, in-app rules editor — all deferred;
  Files/Skills/Rules tabs read as thin wrappers the reviewer didn't find
  compelling versus just using your own editor.
- **Session-vs-worktree-session distinction** (source 3): our MVP's
  `sessionmgr new` in `SCOPE.md` currently assumes every session gets its
  own worktree. Xirp instead offers plain same-directory sessions *and*
  explicit worktree sessions as separate actions. Worth deciding
  deliberately rather than defaulting — this repo's MVP already leans
  "always isolate," which is arguably simpler and safer than replicating
  the option, but call it out as a conscious deviation, not an oversight.
- **tmux-backed session persistence independent of the manager process** —
  a good pattern to adopt regardless of licensing concerns: spawn agent CLIs
  under `tmux` (or, on Windows, an equivalent persistent-session mechanism —
  there is no native tmux; `rustils`' Job Object work already gives us
  process-survives-parent semantics, but a Windows analogue for
  reattachable terminal sessions still needs research) so closing the
  manager UI doesn't kill in-flight agent work.
- **Session hooks + external webhook extensibility** — a small, clearly
  non-proprietary pattern (agent CLI hook → HTTP POST on
  stuck/complete/subagent-done) that's cheap to build and directly answers
  the "no remote/mobile visibility" gap both this project and Xirp share.
  Reasonable v1.1/v2 candidate, and notably something Xirp itself does
  *not* solve natively (source 3 had to bolt on Zapier).
- **No redaction of session data** is called out as a gap in Xirp itself
  (source 3) — if this project ever ships a hook/webhook feature like the
  one above, treat basic secret-scrubbing before any outbound payload as a
  design requirement from day one, not a nice-to-have (this repo's own
  CLAUDE.md: "never commit or log secrets... validate external input at the
  boundary" applies just as much to outbound telemetry as inbound).
- **Check Solo (by Aaron Francis) before building further** — named in
  source 4 as prior art at the same individual-developer scale this
  project targets. If it already covers worktree-per-session multi-agent
  management, our differentiator narrows to "Windows-native" specifically,
  which is a legitimate scope but worth stating explicitly rather than
  discovering the overlap after implementation starts.
- **Cost/model routing** (source 4, unverified hands-on) is a plausible
  v2+ idea — letting a session's model/provider be swapped for
  cost/performance reasons — but given it's not independently confirmed by
  any hands-on source in this doc, it should stay out of the MVP and not be
  taken as a validated must-have feature.

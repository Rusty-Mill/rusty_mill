<!-- Not binding. Exported 2026-11-12 from the Claude Doc https://claude.ai/artifact/Fr2X16gcc9NJJQDPRMzKpQ (rev 13) as the source this pack was built from. Decisions live in ../DECISIONS.md; where they differ, DECISIONS.md wins. The embedded architecture diagram did not survive export (see ../OPEN_QUESTIONS.md Q9). -->

# Our skills tool: recommendations and build plan

Nov 12, 2026 · @Nano Hurtz

Build our own tool as a small Rust core with a CLI first and a desktop shell later. Take Skills Manager's linking model and safety rules, plus Skills Desktop's scanner rules and offline catalog idea. Skip the vendor coupling, telemetry and breadth that neither of us needs.

## Recommendation

Build, don't fork. Reuse ideas from both, and code only from Skills Manager.

- **Why not fork Skills Manager:** it is the better product, but the core logic is spread over two oversized modules (`marketplace.rs` 4.4k lines, `usage.rs` 2.6k). It carries vendor plumbing we would delete on day one: OAuth server, ClawHub publishing, Feishu feedback, donation assets. It also ships a full React editor. A fork means owning 29k lines of someone else's shape.
- **Why not fork Skills Desktop:** it has no LICENSE file, so its code can't be reused. It is also built around one folder (`~/.claude/skills`) and links whole folders, which is the wrong model.
- **Why a fresh core is cheap:** the valuable part is small. A skill model, a source list, a linker that is safe on Windows, a sync check and a scanner come to about 3k to 5k lines of Rust. The hard-won details (junction fallback, managed-path markers, the per-tool path table) are documented in our teardowns, and Skills Manager's MIT license lets us copy those pieces with attribution.
- **Why CLI-first:** your skills already live in git (`skill_pack`, symlinked into `~/.claude/skills`), and agents can drive a CLI or MCP server. A GUI is a view over the same core and can come later as a thin Tauri shell, matching the Project Forge pattern of a headless Rust core.

Evidence: [Skills Manager breakdown](https://claude.ai/artifact/KktUuhC1eFPVrefKYJGPzc), [Skills Desktop breakdown](https://claude.ai/artifact/2sPPUqZevip7az3NBjWhjP), [teardown spec](https://github.com/baileyrd/reverse/blob/claude/project-thread-rni80a/targets/skills-manager-2.2.0/spec.md).

## From Skills Manager

Skills Manager gives us the core model: one library, linked per skill and per tool. Most of its machinery is worth keeping in a smaller form.

| Capability | Verdict | Why |
| --- | --- | --- |
| Per-skill, per-tool links | Take | The right level of control: a skill can be on for Claude Code and off for Codex. Whole-folder linking can't express that |
| Link fallback: symlink, then junction, then marked copy | Take | Windows without Developer Mode can't make symlinks; junctions need no admin rights. The copy fallback keeps it working everywhere |
| Managed-path marker and refusal to delete unmanaged folders | Take, and close the iFlow hole | The single most important safety rule. Apply it to every code path, including copy mode (their iFlow bug, spec §8.5) |
| Tool table: id, config folder, skills folder, CLI name, project folder | Take as data | Their 32-tool table is the best research available. Ship it as a TOML file so adding a tool is a data change, not a release |
| Project bindings | Take | Per-repo skill sets are a real need. Keep their verified list of tools with project-level skill folders |
| Sync check and fix (`doctor`, `fix --yes`) | Take | Drift is inevitable when tools and users touch the folders. A dry-run report plus an explicit fix is the right UX |
| Adopt existing skills | Take | Day-one onboarding: find skills already in tool folders, bring them under management, link back |
| `skm` CLI shape (init, adopt, list, enable, disable, doctor, fix) | Take | Already CLI-first and small; a good starting command set |
| Risk scanner families and modes (off, basic, deep) | Adapt | Keep its context-aware parser: it knows whether a line sits in a markdown example, a comment or docs/, which cuts false positives. Merge in Skills Desktop's broader categories and make rules data |
| Export/import zip with manifest | Adapt, later | Git is our primary transport; zip is for sharing with people who don't use git |
| Marketplace (ClawHub, GitHub URL install) | Adapt | Keep install-from-git-URL. Treat ClawHub as one optional source adapter, not a built-in |
| Usage stats from agent hooks | Adapt, opt-in only | Knowing which skills fire is valuable. Their default-on hook install on every launch is not acceptable |
| LLM translation | Skip | Not a need for us; adds a provider and secrets surface |
| Built-in Monaco editor and file ops | Skip | You already edit skills in your editor; "open in editor" is enough |
| OAuth sign-in, ClawHub publishing, Feishu feedback, updater | Skip | Vendor plumbing with no value outside their service |

## From Skills Desktop

Skills Desktop contributes ideas, not code: its repo has no license. Two ideas are better than Skills Manager's equivalents.

| Capability | Verdict | Why |
| --- | --- | --- |
| Security rule coverage (55 rules in 8 categories, Unix and Windows variants) | Take the coverage, write our own rules | Broader than Skills Manager's 40: it adds secrets, sensitive file access, command injection and Windows-specific destructive commands. Without a license we rewrite the rules ourselves from the categories |
| Configurable skip list for scanning (hidden folders, node\_modules, \_\_pycache\_\_, target) | Take | Keeps vendored dependencies from swamping results. Both apps scan scripts as well as SKILL.md |
| Bundled binary check (exe, dll, so, dylib, jar, msi) | Take | Cheap and high-signal: a skill has no reason to ship a compiled binary |
| Score from 0 to 100 with bands and hard-block rules | Take | Gives one number to sort and gate on. Hard-block rules stop install outright |
| Scan before install (they have the flag, never wired it) | Take, and actually wire it | The point of a scanner is gating installs, not a report page nobody opens |
| Offline catalog snapshot | Adapt | Fast search with no network is nice. Ship it as an optional, separately updated index, not 68 MB inside the binary |
| Sparse shallow `git clone` for installs | Adapt | Right instinct for git-hosted skills. Use a git library or the `git` CLI behind a port so tests don't need the network |
| Native vs linked agent split | Adapt | Some agents read `~/.claude/skills` themselves (Copilot, Cursor, OpenCode, Antigravity, Amp per their table). Recording that avoids duplicate links. Verify each claim before trusting it |
| Whole-folder linking | Skip | All-or-nothing per agent, and it replaces the agent's own folder |
| `fetch_api` generic HTTP proxy for the UI | Skip | An unrestricted HTTP client exposed to the webview. Only typed, purpose-built commands cross the boundary |
| Install reporting to skills.lc | Skip | Telemetry without opt-out |

## Leave out of both

These are design mistakes, not missing features. Each one is a rule for our tool.

| Mistake | Seen in | Our rule |
| --- | --- | --- |
| Webview with no CSP and wide file access (`$HOME/**`) | Both | Strict CSP; the UI gets typed commands only, no file or HTTP passthrough |
| Secrets in plaintext config or localStorage, keys in logs | Both | OS keychain for tokens; never log a secret, even truncated |
| Writing to other tools' config without asking | Skills Manager (hooks on every launch) | Every change outside our own folders is opt-in, previewed and reversible |
| Silent overwrite on reinstall | Both (iFlow enable, Desktop reinstall) | Never delete a folder we can't prove we own; refuse and explain instead |
| Telemetry or vendor endpoints baked in | Both | No outbound calls except to sources you configure |
| Features advertised but unreachable or half-wired | Skills Desktop (scanner, update check) | No feature lands without a test that drives it end to end |
| Logic in two giant files | Both | Small crates with one job each; domain code has no I/O |

## What neither does

These are the reasons to build our own rather than adopt either one. Each fits how you already work.

1. **Git repos as sources, in place.** Point the tool at `skill_pack` (or any repo or folder) and link skills straight from your checkout. Both apps copy skills into their own library, which breaks the edit, commit, push loop you already use.
2. **Profiles.** Named skill sets ("rust work", "reverse engineering") applied to a tool or a project in one command. This generalises Skills Manager's project bindings.
3. **Lint and eval hooks.** Check frontmatter, description length and file size on every adopt or install. Run the `skill_pack` Eval-Kit suite when a skill has one.
4. **Context cost.** Report token weight per skill and per profile, so you see what each enabled skill costs every session.
5. **MCP server.** Expose list, enable, disable, search and scan over MCP, so agents can manage their own skills under the same safety rules.
6. **Plan, then apply.** Every mutating command prints a plan first and applies only with `--yes`; `skm fix` already shows this works.
7. **One scanner for both worlds.** Skills Manager's parser plus the broader rule coverage, as data, gating every install, with SARIF output for CI.

## Architecture

A modular monolith in one Cargo workspace. The core holds the model and the rules and never touches the disk or network. Adapters do the I/O behind traits, so the core is tested with fakes and the Windows linking quirks stay in one crate.

&#91;embedded content: architecture · 3 front ends, 1 core, 4 adapters\]

Front ends never call adapters directly; everything goes through the core, so the CLI, MCP and desktop all get the same safety rules.

**Domain model (core crate):**

| Type | Holds | Rule it enforces |
| --- | --- | --- |
| `Skill` | id, name, description, path, source id, content hash | A valid `SKILL.md` with frontmatter, or it isn't a skill |
| `Source` | id, kind (git repo, folder, remote index), location, enabled | Skills are linked from where they live; nothing is copied by default |
| `Tool` | id, skills folder, project folder, link mode (link, copy, native) | Loaded from a TOML catalog; custom tools are entries in the same file |
| `Profile` | name, skill ids, target tools, optional project | One named set applied in one plan |
| `Link` | skill, tool, path, method (symlink, junction, copy) | Only links carrying our marker are ever replaced or removed |
| `Plan` | ordered actions (link, unlink, adopt, install) | Every change is a plan first; applying needs explicit consent |
| `Finding` | rule id, severity, file, line, context | Hard-block findings stop an install |

## Build plan

Five milestones. Each ends with something you can use daily, and each has a test-backed exit check. No dates yet: sizing waits on the open questions below.

1. **M0: Core and linker.** Workspace with core, linker, store and CLI crates. Tool catalog as TOML seeded from Skills Manager's 32 tools. `list`, `enable`, `disable` with symlink, junction and marked-copy fallback.
   - Exit: enable and disable round-trip on Linux and Windows CI; a test proves an unmarked folder is never deleted (the iFlow case).
2. **M1: Sources and adoption.** Register `skill_pack` and plain folders as sources, linked in place. `adopt` brings existing tool-folder skills under management. `doctor` and `fix --yes` for drift.
   - Exit: your real `~/.claude/skills` setup is reproduced from `skill_pack` with one plan; `doctor` reports clean.
3. **M2: Scanner and gated install.** Rules as TOML, merging both apps' categories, plus the binary check and the 0 to 100 score. Install from a git URL runs the scan first and stops on a hard block. SARIF output.
   - Exit: a fixture set of good and malicious skills scores as expected; a hard-block fixture can't be installed.
4. **M3: Profiles, projects and lint.** Named profiles applied per tool or per project. Frontmatter and size lint, token cost per skill and profile, an Eval-Kit hook where a suite exists.
   - Exit: switching profile in a repo changes only that repo's `.claude/skills`, shown in the plan first.
5. **M4: MCP server, then desktop.** MCP tools for list, enable, disable, search and scan, under the same plan and consent rules. Then, if still wanted, a thin Tauri shell over the core.
   - Exit: an agent session enables a skill through MCP and the change shows in `doctor`.

Optional later: opt-in usage stats from agent hooks, zip export/import, a remote catalog source (ClawHub or an offline index).

## Licensing and provenance

| Source | License | What we may do |
| --- | --- | --- |
| Skills Manager ([jiweiyeah/Skills-Manager](https://github.com/jiweiyeah/Skills-Manager), v2.2.0) | MIT, LICENSE file present | Copy or adapt code (linker fallback, tool table) with its copyright notice kept in a `THIRD_PARTY` file and a comment at each reused piece |
| Skills Desktop ([Harries/skills-desktop](https://github.com/Harries/skills-desktop), 1.3.5) | No LICENSE file; README claims MIT | Ideas and categories only. Write our own rules and code. If the author adds a LICENSE file, revisit |
| Our teardown notes (`targets/skills-manager-2.2.0`) | Ours | The spec is the handoff; implementers work from it, not from decompiled output |

## Open questions

- [ ] **Name and home repo.** A new repo, or a crate in an existing workspace such as rusty\_mill?
- [ ] **House crates.** Should HTTP and async use `rusty_request` and `rusty_tokio`, as rusty\_mill does, rather than reqwest and tokio?
- [ ] **Desktop at all?** Keep M4's Tauri shell, or stop at CLI plus MCP?
- [ ] **Which tools matter.** Ship all 32 catalog entries, or start with the ones you use (Claude Code, Codex, Cursor, Gemini CLI) and verify each before adding more?
- [ ] **Windows priority.** Is Windows a first-class target from M0 (junction fallback tested in CI), or macOS and Linux first?

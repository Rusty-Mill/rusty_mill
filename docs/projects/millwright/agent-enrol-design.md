# Design note: `agent_enrol` (project-lead agents mint their own workers)

Status: **built in #1664 (Nano, 2026-11-13: design approved, build now).** Where the build differs from this note is under "As built" at the end. Nano asked for a note first (2026-11-13). It is
step 2b of the agent-identity work; step 1 (instance labels, #1658/#1660) is live and the
per-agent credential runbook (`per-agent-credentials-runbook.md`) is the config path today.
Belongs with Millwright O4.

## Problem
Settings > Agents is admin-only and manual. A project lead agent (for example
`rocket-league-lead`, limited to `rocket_league`) cannot give its own QA or builder
instances separate, revocable, board-verified credentials. A step 1 label is self-declared,
so it proves nothing.

## What it adds
- New capability `enrol` (code, in `Capability::ALL`; roles stay data, so you grant it by
  editing a role). Not in any role by default.
- MCP/API op `agent_enrol {label, role, projects?}`. The caller must be authenticated by a
  credential (never a caller-named actor, same rule as `board_number_cards`).
- Effects: a new credential in the same file, `change_creds`' checks all apply (copy,
  keep-an-admin, save, swap). Token returned once and never stored elsewhere.
- Also `agent_enrol_revoke {label}` for the caller's own children only. Admin keeps
  everything in Settings.

## Rules (the ceilings)
1. **Role ceiling:** child role's capabilities must be a subset of the caller's. The child
   can never hold `enrol` or `admin`, so the tree is two levels deep.
2. **Project ceiling:** child projects must be a subset of the caller's. A caller with no
   project limit may pass any known project; a caller with a limit must pass `projects`
   (omitting it would inherit "all", which is refused).
3. **Roles are not invented:** the child's role must already exist. Defining roles stays
   admin-only.
4. **Cap:** at most N children per lead (suggest 8), so a runaway agent cannot fill the
   credentials file.
5. **Name:** the child is named from the lead and label, fixed by the board, not chosen
   freely (below).
6. **Audit:** each mint, revoke and refusal is a board event with lead, child, role and
   projects. The token is never in the event.
7. **Revoke:** removing a lead removes its children; a child can be removed alone.

## Naming (the one real wrinkle)
Credential actors may not contain `/` (`valid_actor`), and `/` is what step 1 labels use.
If a child were named `lead/label` it would collide with the lead's own self-declared label
of the same text. Proposal: children are `lead.label` (dot is already allowed), the
credential file gets an optional `"parent": "lead"` field that the ceiling and cascade
checks read, and the UI groups children under their lead. Labels under a child still work
(`lead.label/<short id>`).

## Cost and risks
- About 60-100 lines plus tests in `identities.rs`, `settings.rs` and `api.rs`, one change
  fragment, one PR for dottiusmaximus review (auth path: independent review wanted).
- **Rollback hazard:** the credentials file gains a `parent` field and the parser is
  `deny_unknown_fields`, so the previous binary would refuse a file that holds children.
  Rolling back means removing children first (or accepting the field in a preceding
  release). State this in the deploy prompt.
- **Trust:** whoever holds `enrol` can mint tokens up to its own ceiling without a human
  approving each. Mitigated by the ceilings, the cap and the audit event; the grant of
  `enrol` itself stays yours.
- **Does not help** cloud, chat and voice sessions on the claude.ai connector: one OAuth
  grant per account, no per-session token. They stay on step 1 labels.

## Open questions for Nano (recommendation first)
1. Child naming `lead.label` with a stored `parent` field? **Yes.**
2. Cap per lead? **8.**
3. Should `enrol` be limited to roles you list per lead (an allow-list) rather than any
   role at or below the lead's capabilities? **No for v1; the subset rule is enough.**
4. Build now or wait until a project lead agent actually exists? **Wait: no caller today.**

## Answers (Nano, 2026-11-13, "Concur" with all four recommendations)
1. Child naming `lead.label` with a stored `parent` field: yes.
2. Cap per lead: 8.
3. No per-lead role allow-list in v1; the subset rule is enough.
4. Wait until a project lead agent actually exists; no card until then.

## As built (#1664)
- Arguments are `agent_role` and `agent_projects`, because `role` already names the caller's own
  role in the shared `Who` arguments. Unknown arguments are refused.
- HTTP `/api/agent_enrol` and `/api/agent_enrol_revoke` only, not MCP tools, matching "settings are
  not MCP tools". Exposing them over MCP needs `Settings` wired into `mcp::server`; not decided.
- **Bearer token only (Nano, review 2026-11-13):** an OAuth sign-in is refused on both calls, so an
  approved app cannot mint raw tokens that revoking the app would not take back.
- **Prune on demotion (Nano, review 2026-11-13):** when an admin changes a lead's role or projects, or
  edits roles, minted agents that no longer fit under their lead (a capability it lacks, `enrol` or
  `admin`, or wider projects) are removed, logged, and listed as `removed` in the response.
- Audit is a log line, not a board event: events belong to cards.
- Removing a minted agent does not revoke OAuth grants made under its name; minting the same label
  again revives them (as with an admin remove-then-add).
- Rollback: an older binary refuses a credentials file with `parent` and a roles file naming
  `enrol`; remove the minted agents and the `enrol` role first.

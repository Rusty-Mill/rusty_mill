# #1642 Wire Let Em Loose end to end: scope and slices

Status: draft for Nano's approval (D25). No code, no host change. The host choice (section 2) is waiting on Nano's answer to the decision card of 2026-11-28; this note is updated with it.

## 1. Goal and what is already built

Done when (card): one run goes from intent through the board and BBP claims to a decision in the inbox that Nano approves, reachable without SSH to BaileyAi.

Built and merged:
- BBP host, orch and the inbox board on BaileyAi (#1092, `1092-deploy-layout.md`): a second Mill Board on `127.0.0.1:8787` with its own SQLite file, the `mill-board` user and the `mill-board-bbp-guard` sudo rule (D132, D175).
- Inbox: `inbox_list` and `inbox_act` over `bbp inbox`/`bbp human`, revision fence and `dry_run` (#595, D35, D173).
- LEL tab read endpoints: `lel_claims` and `lel_spend` over the read-only `bbp claims` verb (D220, D221). `lel_intent` and brief text are deferred (D221).
- The one approved paid run: the #1318 toy run (`1318-toy-run.sh`, D167, D197).

The gap: the live board (the claude.ai connector, on host `remindme`) has no BBP host, so its `inbox_list` answers "the inbox has no BBP host". The inbox board on BaileyAi is loopback only. That is by design (D175), so the missing piece is reach, not code.

## 2. Where the inbox and BBP host live (Nano decides)

Options put to Nano, from let-em-loose-expert's escalation:
- **a) Expose the BaileyAi inbox board as a second claude.ai connector (recommended).** No code change. claude.ai calls connectors from Anthropic's cloud, so tailnet-only is not enough: it needs Tailscale Funnel or a reverse proxy with HTTPS, the board's OAuth sign-in and `MILL_BOARD_ALLOWED_HOSTS`. rusty_term reaches it over the tailnet. Keeps D35, D132 and D175; adds no trust path into `orch-runner`. It is a host change on Nano's machine (ask-first, D128; D131 does not cover it).
- **b) The live board on remindme gets a remote BBP path** (proxy to the BaileyAi board's API, or a new bbp transport). New design and a public API change; close to the remote path D175 rejected.
- **c) Move the live board to BaileyAi.** One board for cards and inbox. Reverses D175.

Moving BBP to remindme reverses D35, D53 and D131 and was not offered.

Under (a) the live board's "no BBP host" error should name the inbox connector instead (a one-line wording change, its own PR).

## 3. Slices

One small PR or prompt each. Child cards under #1642.

1. **This note** with Nano's host answer and design approval.
2. **Deploy the chosen host**, as a paste-ready prompt for Nano (D94). Under (a): HTTPS exposure, OAuth and allowed hosts for the inbox board; the guard edit that admits `claims` (D220) so `lel_claims` stops answering "verb refused"; the #1551 anchor guard line and timer only if Nano authorises them in the same step (D224). Every BaileyAi change needs his authorisation by name.
3. **LEL tab read endpoints: finish.** Mostly done (D220). Left: #1416 slice 5 (PR #1961) and the later #1416 slices. Runs alongside 1 and 2; tracked on #1416, not duplicated here.
4. **One end-to-end run.** No new paid run: the #1318 toy run is the only approved one (D167; `no_overage` lapses 2026-12-25). For #1642 its escalation must reach the reachable inbox and Nano approves it with `inbox_act` from claude.ai. Board seeding reuses `1166-seed-inbox.sh` and `1166-smoke-run.md`. Depends on #1318, slice 1 and slice 2; not on #1320 (not approved, D167).

## 4. Open points (to let-em-loose-expert before slice 4)

- The toy run makes its own BBP dir and leaves `/srv/lel/bbp` and the inbox board untouched (`1318-toy-run.md` step 5), while the inbox board's guard allows only `/srv/lel/bbp`. Slice 4 needs either the toy task opened in `/srv/lel/bbp` or a read-back that points the inbox at it. Either touches an approved runbook.
- The toy claim needs an escalation for Nano to approve; the runbook's pass rule does not require one.
- Whether any lens run on the path is a paid call stays ask-first.

## 5. Out of scope

Sandbox and jail layers (#726, #1412), the token minter (#1382), Agent Cloud. Agents run uncontained (decision 2026-11-18).

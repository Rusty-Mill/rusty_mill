# Goldfish Review Report

**Target:** rev 2 of `design.md` (2026-11-04); rev 3 answers these findings in its section 11
**Code checked:** `/home/user/rusty_mill` working tree (crates/apps/rusty_mill_board, rusty_bbp_host, libs/protocol/rusty_bbp, foundation/rusty_pk)
**Lenses:** design critic, zoom-out/simplicity, implementation readiness, codebase details.
**Process note:** this session had no subagent-spawning tool, so the synthesizer applied each lens in a single pass rather than running independent Goldfish instances. Each finding below was checked against the source cited.

Totals: 2 Blocker, 10 Major, 9 Nit.

## Prioritized Findings (Blockers -> Major -> Nits)

### B1. The duty ledger is not durable, so R3/R4 fail open after a restart - Severity: Blocker
* **Reviewer Lens:** `codebase_details`, `design_critic`
* **Affected Scope:** §4 R2 ("The ledger is the card's event log; no new store"), §5.3, §6
* **Discrepancy Description**: The production store is `TickStore`, and `tick.rs:27-30` says: "Events are not durable: rusty_tick has no change log. They are kept in memory for this process." After a board restart a card has no recorded authors and no passing review. That means R3 ("reviewer is an author") never fires, and R4 cannot find a review, while `Approved` and `Revoked` records and `board_approvals` disappear. `Event` (`domain.rs:455-470`) also has no payload fields for gate, rev, verbatim or source.
* **Rationale & Recommendation**: Store authors, review verdicts and approvals on the card's durable `board` document. It is already fenced with the task version (`tick.rs:10-25`). Drop "no new store". If the ledger is missing, fail closed: refuse the review or gate and name the reason.

### B2. `bbp human --op` writes the `by:` label directly and bypasses both `--as` and receipts - Severity: Blocker
* **Reviewer Lens:** `codebase_details`, `design_critic`
* **Affected Scope:** §1 (BBP bullet), §7 "Owned tasks"
* **Discrepancy Description**: `bin/bbp.rs:483,502` passes a caller-supplied `--op` to `human::command`, which uses it as is (`human.rs:346-349`). Only the `None` branch goes through `labelled_op`. Any caller can therefore write `--op by:<approver>/x`. Under §7 this is indistinguishable from a receipt-backed `by:<approver>` op. `report.rs:55` and `bbp metrics` trust that label.
* **Rationale & Recommendation**: For owned tasks, refuse a caller `--op` with a `by:` prefix, or make the host derive it from the verified receipt. Write the receipt into the log, or at least its hash and key id. Then `metrics` can check the signature instead of trusting the label. Add a test that a forged `--op` is refused.

### M1. R5 is bypassed in two moves through V&Ving -> Building -> Done - Severity: Major
* **Reviewer Lens:** `design_critic`
* **Affected Scope:** §4 R5, §3 maker row
* **Discrepancy Description**: The maker role holds `test` and `triage`. `factory.workflow.json:19` allows V&Ving->Building by `test`, and `triage` waives the claim (`domain.rs:654-659`). `factory.workflow.json:23` allows Building->Done by `close`. A maker can therefore close a card that reached V&Ving without the merge approval R5 requires. `card_add` with `done: true` (README:42-44) also creates Done cards with no approval.
* **Rationale & Recommendation**: Apply R5 to every `close` into Done, or refuse Building->Done for a card that has ever reached V&Ving. Say that `done: true` is bookkeeping and outside the gate.

### M2. Unblocks launder a card past the review requirement - Severity: Major
* **Reviewer Lens:** `design_critic`
* **Affected Scope:** §4 R4 ("Unblocks need only `approve`")
* **Discrepancy Description**: Any open card may enter Blocked. `factory.workflow.json:24-28` lists unblocks to Ready, Planning, Architecting, Building and V&Ving. Planning->Blocked->Building therefore needs only `approve`, with no passing design review. Nano is shown "unblock to Building", which does not reveal that the design gate and its review are being skipped. The V&Ving->Done review check catches the code side. Nothing catches the design side.
* **Rationale & Recommendation**: Allow an unblock only to the column the card left (record it when the card is blocked), or apply R4's review condition based on the destination.

### M3. R3 leaves openai-made work with no eligible reviewer - Severity: Major
* **Reviewer Lens:** `design_critic`, `implementation_readiness`
* **Affected Scope:** §3 table, §4 R3
* **Discrepancy Description**: Both listed reviewers, Dots and Codex, are `openai`. Cards made by Codex (openai) or by Hermes on openai share a provider with every reviewer, so R3 refuses all of them. Those cards deadlock in Building.
* **Rationale & Recommendation**: Add a reviewer that is not on openai (an anthropic or google credential), or state that openai makers are not used until one exists.

### M4. Under recommended option C, the maker session can approve its own design gates - Severity: Major
* **Reviewer Lens:** `design_critic`, `zoom_out`
* **Affected Scope:** §5 "Known weak point", §10 D1, §4 R4
* **Discrepancy Description**: R4's "approver is never an author" is always true, because the approver is always `nano-proxy`. The real actor is whichever Claude surface calls the connector, and that can be the claude-cloud session that made the card. C keeps exactly that path open for Backlog->Ready and Architecting->Building. Separately, an OAuth grant acts as whatever token was pasted at consent (`web.rs:5-10`). If Nano's existing Mill_Board connector on claude.ai was consented with the `human` token, every Claude surface already holds `human` and `admin`. In that case R1-R6, option B and D2 are all moot.
* **Rationale & Recommendation**: Check which credential Nano's current claude.ai board connector uses, and state the answer in §1. Present the self-approval path for design gates under A and C plainly in D1, or recommend B for all gates.

### M5. The BBP inbox and claims-mode gates are left out of both the `admin` change and receipts - Severity: Major
* **Reviewer Lens:** `codebase_details`, `design_critic`
* **Affected Scope:** §2 ("`admin` becomes settings-only"), §7 ("Other human verbs are unchanged")
* **Discrepancy Description**: The #595 inbox carries BBP human authority behind `admin` (`api.rs:144-150`) and runs `bbp human --as <actor>` (`bbp_cli.rs:45-60`). Its Approve maps to `claim accept` for `Irreversible` escalations and to budget extends (`inbox.rs:541-555`). So `admin` is not settings-only. Directly through `bbp human`, the claims-mode gates `claim accept` (irreversible actions) and `holdout-updated` (adopting changed tests, #597 G3) stay unsigned and can be self-approved.
* **Rationale & Recommendation**: State that the inbox stays on `admin`, which after D2 means human only. Bring `claim accept` on irreversible escalations and `holdout-updated` under the receipt rule, or list them explicitly as accepted gaps.

### M6. Cards can be moved in rusty_tick without any board check - Severity: Major
* **Reviewer Lens:** `zoom_out`, `codebase_details`
* **Affected Scope:** §4 (scope of "enforced by the board")
* **Discrepancy Description**: Cards are rusty_tick tasks. Column and claim are `board:`/`claim:` tags, and a closed task reads as Done (`tick.rs:4-8`). Closing a task or retagging it through rusty_tick involves no capability or duty check. The board sees it only afterwards, as actor `tick`. The Tick connector is on Nano's account, so it is reachable by the same Claude surfaces.
* **Rationale & Recommendation**: Name this in §1 and §8 as outside the board's enforcement. Alternatively, have the board flag or refuse to honour a `tick`-originated change that crosses a gate.

### M7. §5.3 checks that cannot be built as written - Severity: Major
* **Reviewer Lens:** `implementation_readiness`, `zoom_out`
* **Affected Scope:** §5.1-5.3
* **Discrepancy Description**: "The request is at most 1 hour old" has nothing to measure: `card_approve` is a single call, and the readback happens only in conversation. "Rev unchanged since the readback" is just the caller-supplied CAS. "Listener session" is self-asserted. Every comment or sync also bumps rev (`domain.rs:863-871`, test at `domain.rs:1221`), so a bot comment between readback and approve makes the approval fail.
* **Rationale & Recommendation**: Cut the 1-hour check and keep the rev fence. Label `source` and `session` as unverified audit data. Note that comments invalidate a readback.

### M8. The merge gate is not enforced by the board - Severity: Major
* **Reviewer Lens:** `codebase_details`, `design_critic`
* **Affected Scope:** §5.4, §5 "GitHub side"
* **Discrepancy Description**: The board knows nothing about PRs. `github.rs:177` filters PRs out, and `Card` has `issue` but no PR link. "Bound to the PR head" is therefore a free string, and "a new push lapses it" cannot be detected. Enforcement rests on dottiusmaximus remembering to check.
* **Rationale & Recommendation**: Either say plainly that this is dottiusmaximus's discipline, or add a PR link to the card plus a required GitHub check that queries the board for an approval of the head. The design already wants small, so the plain statement is probably enough.

### M9. The §7 receipt is underspecified and bound to the wrong thing - Severity: Major
* **Reviewer Lens:** `implementation_readiness`
* **Affected Scope:** §7 "Owned tasks", §9 step 2
* **Discrepancy Description**: "Same checks as section 5" cannot apply as written, because a BBP task is not a board card. The board would have to read task state through `bbp` (`bbp_cli.rs`). BBP rev moves with every event, so a receipt bound to "rev = current" goes stale whenever any agent writes. `approve-plan` and `approve-merge` already name the subject (`art`, plus `run` for merge, `human.rs:70-79`), and `--fence` already exists. Also unspecified: the receipt encoding, the expiry length, where the board's private key lives and how it rotates, and the requirement that the hosting config holding the public key be outside what the moderator-operating agent can write.
* **Rationale & Recommendation**: Bind the receipt to task, gate and subject `art` (and `run`), with an expiry, and use `--fence` for staleness. Add one line each on key custody and on config write protection.

### M10. "Undo" needs moves the factory workflow does not have - Severity: Major
* **Reviewer Lens:** `implementation_readiness`
* **Affected Scope:** §6 "Undo"
* **Discrepancy Description**: `factory.workflow.json` has no Ready->Backlog and no Building->Architecting. "He moves the card back" therefore needs Blocked plus a gated unblock, or movement rules switched to Open.
* **Rationale & Recommendation**: Make revoke one op that records `Revoked` and returns the card to the gate's source column, or add the two backward transitions.

### Z1. Drop `--gates sandbox` and the `task_opened` field - Severity: Major (simplification)
* **Reviewer Lens:** `zoom_out`
* **Affected Scope:** §7 "Layers", "Sandbox"
* **Discrepancy Description**: Enforcement is in the host, and `TaskOpened` already records `repo` (`event.rs:106`). The host can decide unsigned-allowed from the hosting config's sandbox repo list alone, with no engine change, no flag, no new log field and no metrics field. "Sandbox" also already means OS confinement (`--confine sandbox`, `bbp __sandbox`, `bin/bbp.rs:7-33`), and reusing a reassuring word for "unsigned gates" invites misuse.
* **Rationale & Recommendation**: Make the rule "repo listed in the hosting config: gates take `--as`; otherwise a receipt is required." If a per-task mode is kept, rename it (for example `unowned`).

### N1. `card_review` and the "newer than the newest author claim" rule are implicit - Severity: Nit
* **Reviewer Lens:** `zoom_out`, `implementation_readiness`
* **Affected Scope:** §4 R3/R4, §5.4
* **Discrepancy Description**: The architect's claim in Architecting always comes after the Planning->Architecting review. The Architecting->Building gate therefore always needs a `card_review`, which the design does not say. Under §5.4 the gated V&Ving->Done `merge` transition is never used, because `close` makes the move.
* **Rationale & Recommendation**: State that architecture review is `card_review` in Architecting. Remove the dead gated `merge` transition from the factory workflow.

### N2. `board_approvals` and the Settings view duplicate `board_events` - Severity: Nit
* **Reviewer Lens:** `zoom_out`
* **Affected Scope:** §6, §9 step 1
* **Rationale & Recommendation**: Filter `board_events` by kind. Cut the new tool and the view.

### N3. §1 role claims describe live settings, not the code - Severity: Nit
* **Reviewer Lens:** `codebase_details`
* **Affected Scope:** §1 "Roles"
* **Discrepancy Description**: `Roles::default` (`board.rs:45-63`) has builder = build, close, project and reviewer = review only. The "all ten" and "review+merge" in §1 must come from the live `roles.json`. `open_credentials` requires a `human` role holding `admin` (`settings.rs:207-216`).
* **Rationale & Recommendation**: Say "live roles.json", and keep `human` holding `admin`.

### N4. Codex in two rows and Hermes per provider need distinct actor names - Severity: Nit
* **Reviewer Lens:** `codebase_details`
* **Affected Scope:** §3
* **Discrepancy Description**: `Credentials::add` refuses a duplicate actor name (`identities.rs:172`). `by_actor`, and with it OAuth, returns `None` for an actor with two credentials (`identities.rs:281-290`).
* **Rationale & Recommendation**: Name the credentials, for example `codex-make` / `codex-review` and `hermes-anthropic` / `hermes-openai`.

### N5. `move_freely` also hard-codes `Admin` - Severity: Nit
* **Reviewer Lens:** `implementation_readiness`
* **Affected Scope:** §2, §9 step 1
* **Discrepancy Description**: The checks for jumping over a gate are at `domain.rs:701` and `domain.rs:705`, and their doc comments (`domain.rs:196`, `513`, `642`, `664-667`; `workflow.rs:77`) say `Admin`.
* **Rationale & Recommendation**: List these in step 1 so that open movement rules do not still require `admin`.

### N6. R2 reads current capabilities, not those held at claim time - Severity: Nit
* **Reviewer Lens:** `design_critic`
* **Affected Scope:** §4 R2
* **Discrepancy Description**: "A maker capability they hold" is evaluated now, so a later role change rewrites who counts as an author. Authorship is also claims only: every agent PR is authored as baileyrd, so commits pushed to a branch the actor never claimed are invisible to R2.
* **Rationale & Recommendation**: Record the author when the claim is made (see B1), and acknowledge the commit gap in one line.

### N7. R4 stops Nano from approving gates on work he authored - Severity: Nit
* **Reviewer Lens:** `design_critic`
* **Affected Scope:** §4 R4, §3 human row
* **Rationale & Recommendation**: State whether the `human` role is exempt. Otherwise the owner's own planning claims deadlock his gates.

### N8. In-flight BBP tasks start requiring receipts on upgrade - Severity: Nit
* **Reviewer Lens:** `implementation_readiness`
* **Affected Scope:** §7 ("Owned is the default and is not written")
* **Discrepancy Description**: Existing logs replay byte for byte, but their next gate answer now needs a receipt.
* **Rationale & Recommendation**: Add one line on migration: issue receipts for open tasks, or list their repos as unsigned until they close.

### N9. "Verbatim is Nano's two lines" is ambiguous - Severity: Nit
* **Reviewer Lens:** `comprehension`
* **Affected Scope:** §5.2
* **Rationale & Recommendation**: Say which two lines: the request and his "Approve?" answer.

# Factory roles, separation of duties and approval proxy (#447), design rev 3

> **Status: approved for build (Nano, 2026-11-25), with the answers in section 12.** Mill Board
> card #447. Section 1 and the code references describe `main` and the live board as of
> 2026-11-04; the build re-checks them.

2026-11-04. Rev 2 was reviewed by a fresh-context Goldfish pass
(`goldfish-review.md`, 2 blockers, 10 majors) and the Let Em Loose expert (BBP gates).
Rev 3 fixes every blocker and major; section 11 maps each finding. Replaces the 2026-10-29
draft (Claude Doc WBXcWFoubjEYiSfwFeg6wz).

## 1. What exists today (main 41016c91 and the live board)

- Columns: Backlog, Ready, Planning, Architecting, Building, V&Ving, Done, Blocked.
- Gates need their capability **and** `admin` (`check_transition`, and `move_freely` at
  `domain.rs:701,705`): Backlog→Ready, Architecting→Building, V&Ving→Done, five Blocked→X.
- Live roles: `builder` and `human` hold all ten capabilities. claude-cloud is `builder`
  (2026-11-02 grant), and every Claude surface on Nano's account uses that connector, so
  **any Claude session can cross every gate, review and merge its own card.**
- The #595 decision inbox approves irreversible-action claims on `admin` (`api.rs:144`).
- Board events are memory-only (`tick.rs:27`); the card's tick document (deps, forge link,
  execution) is durable.
- BBP: `bbp human --as WHO` labels an answer and `--op by:X/..` sets the label directly
  (`human.rs:346`); `bbp metrics` trusts the label. Both are self-asserted.

## 2. Capabilities

Add one: **`approve`**, cross a gate or accept an inbox claim. It replaces `admin` in
`check_transition`, `move_freely` and the inbox; `admin` becomes settings-only. No `deploy`
capability until something deploys through the board.

## 3. Roles and eligibility (Settings data)

| Role (actor) | Capabilities | Provider |
|---|---|---|
| human (`nano`) | all | human |
| approver (`nano-proxy`) | approve | human |
| intake (`duet`) | triage | anthropic |
| maker (`claude-cloud`, `hermes-local`, `hermes-<provider>`) | triage, plan, architect, build, test, close, project, review | per actor |
| reviewer (`dots`, `codex`) | review, test | openai |
| GitHub merger | dottiusmaximus (not a board actor) | — |

- claude-cloud drops `merge` and `admin`; it keeps `review` so it can review openai/local work.
  Every maker can review, and R3 decides whose work.
- One credential per actor and provider; a model on two providers gets two actors.

## 4. Separation of duties, enforced by the board

- **Ledger (durable).** The card's tick document gains `duties`: one entry per claim,
  review and approval, with actor, provider *at that time*, column, kind and card digest
  (title + body). The board appends; no other store.
- **R1 Provider.** Each credential needs `provider`. A review or approve by an actor without
  one is refused (fail closed).
- **R2 Authors.** Ledger claims in Planning, Architecting or Building.
- **R3 Review.** A forward `review` move (Planning→Architecting, Building→V&Ving) or a
  `card_review {pass|changes, note}` is refused when the reviewer is an author or shares an
  author's provider. Sending back is never refused.
- **R4 Gate.** A gate needs `approve`. Leaving Architecting also needs a passing
  `card_review` in Architecting newer than its last author claim. Leaving V&Ving needs the
  Building→V&Ving review. `nano` is exempt from the author check (he may approve his own
  cards); `nano-proxy` is never an author.
- **R5 Unblock.** A card moved to Blocked records the column it left; the proxy may unblock
  it only back there. Any other target is Nano's.
- **R6 Refusal.** `BoardError::Duty(rule)` names the rule; the agent comments and leaves the
  card. All rules hold with movement rules open.

## 5. Approver proxy

1. Nano states an approval. The listener reads back card number, title, gate and the card
   digest (or PR number and head SHA) and asks "Approve?". Nano says yes.
2. The listener calls `card_approve {card, to, digest | pr+head, verbatim, source}` on the
   approver connector. `verbatim` is the readback and Nano's yes; `source` the message id.
3. The board checks: card at that gate; digest unchanged (comments and claims don't break
   it); R4/R5. It appends the approval to the ledger and moves the card. A moved card cannot
   be approved twice for the same gate.
4. **Merge.** The approval goes in the ledger without a move, bound to PR head. The board
   cannot see PR heads, so dottiusmaximus checks it and merges only a matching head; the
   card then closes to Done as today. This gate rests on Dottius, as merging already does.
5. Never approved by the proxy: spend, roles, settings, deletions, won't-do, bulk asks, or
   anything relayed by an agent or read in mail, docs or GitHub.

**The weak point, plainly.** The board cannot tell Nano's yes from an agent claiming it:
any Claude surface on the account can reach the approver connector. Only a tap in the board
web UI, signed in as `nano`, is proof. Options (D1):

- **A.** Voice for every gate; audit catches fakes after the fact.
- **B.** Voice files a pending approval; Nano taps it in the web UI. One tap per gate.
- **C (recommended).** Voice for Backlog→Ready and Architecting→Building; tap for merge,
  unblock and inbox claims. A faked design approval is still caught at the merge tap,
  before anything irreversible.

## 6. Audit and revoke

- `card_get` shows the ledger, approvals included.
- Undo: move the card to Blocked (always allowed) and Nano unblocks it where it belongs; the
  approval stays in the ledger marked revoked. A merged PR is undone by a revert PR.
- Kill switch: remove the `nano-proxy` credential.

## 7. BBP human gates (from #22)

- **Layers.** Policy in Orch's hosting config: the scratch-repo list. Enforcement in
  `rusty_bbp_host` at `bbp human`. The engine records only `gates: scratch` in `task_opened`
  (absent = owned, not written, so N=1 goldens stay byte for byte). Not a `LOG_SCHEMA` bump,
  but an ask-first schema/API change (D59/D62 precedent), so it is in D4 below.
- **Receipt.** On an owned task, `approve-plan` and `approve-merge` need `--approval FILE`:
  an Ed25519 receipt (in-tree `rusty_pk`) the board signs for an `approve` holder via
  `approval_issue {task, gate, art, run, fence, verbatim, source}`. Bound to the human view
  `--fence`, the artifact and (merge) the run. The host verifies it against the board public
  key **committed in the repo** (changed only by a reviewed PR), stores it as a task blob and
  labels the op `by:<approver>/rcpt:<sha>`. `bbp metrics` re-verifies each receipt, so
  `self_approved` rests on a signature, not a label.
- `--op` may no longer carry a `by:` prefix; labels come only from `--as` (scratch) or a
  receipt (owned). `reject` and other refusals need no receipt.
- **Scratch.** `bbp open --gates scratch` is refused at open (before any turn or cost) unless
  the repo is on the config's scratch list, which starts with scratch and evidence repos only.
  Scratch tasks accept `--as` answers; metrics reports `scratch: true`. Not combinable with
  claims mode until D3 settles. Host and orch e2e fixtures open with `--gates scratch`.
- **Claims mode** has no plan or merge gates (addendum lines 10, 250); which of its accept
  verbs need a receipt is D3.
- Mill Board's BBP adapter (`bbp_cli.rs`) and Orch pass `--approval` through.
- In-flight owned tasks: their next gate answer needs a receipt. None are live today.
- **Tests first:** no-receipt answer refused; forged, wrong-fence, wrong-artifact and
  other-task receipts refused; `--op by:` refused; scratch task allows and logs; scratch on a
  listed-real repo refused at open; N=1 goldens untouched; metrics flags an unsigned owned
  answer as self-approved.

**Limit.** A process running as the moderator's user can still edit the store files. The
receipt makes that forgery detectable by `bbp metrics`; it does not prevent it.

## 8. Known gaps, not in this card

- rusty_tick writes bypass the board (an agent with the Tick connector can retag a card).
  Fix: board-only Tick credentials. Proposed as a follow-up card.
- 6 retries then Blocked (Orch), deploy role, executor convergence, #449.

## 9. Build order, after approval

1. Board: `approve` (gates, `move_freely`, inbox), `provider`, ledger in the tick document,
   R1–R6, `card_review`, `card_approve`, tap-to-confirm pending approvals (if B or C).
2. Board `approval_issue` + signing key; BBP receipt check, `--op` rule, `--gates scratch`,
   metrics verification, test updates.
3. Settings data (Nano): roles of section 3, providers, `nano-proxy` credential + connector.
4. #449 end-to-end test.

## 10. Decisions for Nano

- **D1** proxy trust: A, B or C (recommend C).
- **D2** narrow claude-cloud: drop `merge` and `admin`, reversing the 2026-11-02 grant
  (recommend yes; the proxy replaces it).
- **D3** claims-mode verbs needing a receipt: (a) none yet; (b) the accepting ones: `claim
  accept`, `choose`, `holdout-updated`, `extend`, `extend-claim` (recommend b; otherwise an
  agent can accept its own claim); (c) b plus `close`, `rerun`, `receipt`.
- **D4** ask-first API/schema items: `task_opened.gates`, `bbp open --gates`, `bbp human
  --approval`, the `--op by:` refusal, board `card_review` / `card_approve` /
  `approval_issue`, `provider` in credentials.
- Defaults taken unless Nano objects: no deploy capability; card 0917 stays separate; Dots and
  dottiusmaximus are separate actors.

## 11. Review findings and their fix

| Finding | Fix |
|---|---|
| B1 events not durable | ledger in the tick document (§4) |
| B2 `--op by:` bypass | refused; labels from receipts (§7) |
| M3 V&Ving→Building→close | merge gate is Dottius's head check (§5.4), stated as such |
| M4 unblock skips review | R5 |
| M5 all reviewers openai | makers review other providers' work (§3) |
| M6 option C self-approval | stated plainly; merge tap backstops (§5) |
| M7 inbox on admin, claims verbs | `approve` in inbox (§2); D3 |
| M8 Tick bypass | §8 follow-up |
| M9 1h check, rev churn | dropped freshness; digest binding (§5.3) |
| M10 no PR head data | relies on Dottius, said so (§5.4) |
| M11 receipt underspecified | fence + art + run, repo-pinned key, blob, metrics re-verify (§7) |
| M12 undo needs back moves | via Blocked (§6) |
| M13 cut flag, "sandbox" clash | kept per card, renamed `scratch`, in D4 |
| Nits | 14 §4 R4; 15 cut approvals log; 17 actor names; 18 `move_freely`; 19 provider at claim time; 20 Nano exempt; 21 in-flight note; 22 verbatim defined |

## 12. Nano's answers (2026-11-25, project thread message cmsg_014AqbZs58L565hMV2pFkCX5CZatgThTCScc6TZfje7dmZ)

> D1. Voice is enough for the design gates
> D2. Not yet
> D3. None for now
> D4. I Approve them as a set.

What the build does with them:

- **D1 = C.** The proxy crosses Backlog→Ready and Architecting→Building on a spoken approval.
  For the merge gate and unblocks it only files a *pending* approval; Nano confirms it with one
  tap in the board web UI. Inbox claims stay with Nano.
- **D2 = not yet.** claude-cloud keeps its role. Gates take `approve` **or** `admin`, and an
  actor holding `admin` is the owner's override: R3–R5 bind every other actor, and the ledger
  records admin actions too, so self-reviews stay visible. Narrowing claude-cloud later drops it
  from that override with no code change.
- **D3 = none.** Claims-mode human verbs are unchanged; `--gates scratch` is refused with claims
  mode until a receipt rule exists for it.
- **D4 approved:** `task_opened.gates`, `bbp open --gates`, `bbp human --approval`, the `--op by:`
  refusal, `card_review` / `card_approve` / `approval_issue`, and `provider` on credentials.
- **R5 under D1 = C.** Every unblock the proxy files waits for Nano's tap, and the tap shows its
  target, so the build does not record the column a card left for Blocked. The proxy may file
  any unblock; only Nano's confirmation moves the card.
- **Review build notes (PR #1881 review).**
  - A move records an approval only when it went through the gated route. A `close` into Done
    beside the merge gate does not count.
  - Nano's confirmation must match the digest the proxy filed.
  - A filing expires when the card leaves its gate.
  - A recorder's close (`record` counted as `close`) is recorded as the close it is, not as an
    approval.
  - When the owner approves text that was edited after a filing, the filing expires and the
    owner's own approval is recorded, so a stale filing cannot lock the owner out.
  - The merge gate counts the V&Ving verdicts and the Building review. It needs one passing
    verdict and none asking for changes. Each reviewer keeps only their newest verdict.
  - Repeated facts move to the end of the ledger instead of adding entries: an author's
    re-claim of a column, a reviewer's verdict on a column, and a filer's new filing for a
    gate whose earlier filing expired. Only distinct identities still add entries. When the
    ledger is full, others are refused, but the owner can write up to twice the cap.
  - An admin's `card_approve` needs a signed-in credential of its own, not a connector or a
    named role.
  - R4 needs a review of the work the gate rests on: Architecting's own for the design gate,
    the last authored column for V&Ving.
  - A reviewer's newer review of the same work replaces their older one, so reviews cannot
    fill the ledger.
- **R1 while providers are unset.** An actor with no provider is compared by name only, so
  setting providers is what turns on the cross-provider rule. Fail-closed comes with D2.

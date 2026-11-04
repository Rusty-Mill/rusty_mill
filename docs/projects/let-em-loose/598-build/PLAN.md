# #598 build plan (elephant pass, 2026-11-04)

Kept in the repo per Nano's 2026-11-04 rule that design docs live under `docs/projects/let-em-loose/`. The Goldfish review below names D67 and Q14; those defaults are now D78 and Q16.

Card #598 (bbp_factory): close four gaps in BBP/orch. Design approved; all §6 schema/API items approved (DECISIONS D59, D62). The design file `598-gaps-design.md` is not reachable, so shapes not fixed by D59/D62 come from the let-em-loose expert's defaults (D78, `lel:598:build-defaults-without-design-doc`). Nano's answers Q1 to Q6 are on the card.

Constraints: agents open PRs only (dottiusmaximus reviews and merges); no paid API; all new fields optional and not written at their defaults, so existing logs and profile digests stay byte-identical (no migration, D62). Another thread is building the #594 lens runner shell (`rusty_orch lens run`, PRs #1467, #1479); this plan must not duplicate its wiring.

## Already delivered (by #594, #597) — not rebuilt
- `orch-lens`: `DataClass` public < internal < sensitive and `admit`; providers carry `max_class` (cloud capped at internal) and a required `price`; `domain_classes` replaced `sensitive_domains`; `ceilings {experiment_usd, task_usd, task_calls}`; `Meter` reserve/settle/release with `Stop::{Experiment, TaskSpend, TaskCalls}`; orch-store `MeterRecord` (spend + reservations).
- Host: `ProfileSet.baseline` pins every candidate's base; `protected` globs (`protected_path`).
- v0 is flat-rate (ADR-0021): non-zero prices are refused, so money ceilings don't bind in v0; the call ceiling does.

## PR 1 — rusty_bbp + host: intent versions and task pause (gap 3, half of gap 4)
Protocol (claims mode only, schema 5, no bump):
- `HumanAction::IntentRevised { brief: BlobRef, affects: Affects }`. `Affects = All | Some { claims: Vec<MsgId>, domains: Vec<String> }`. Taken while the task is `open` or `escalated`. Both lists empty is `bad_affects`; a claim id that isn't a claim on the task is `bad_affects`.
- Effect, one transaction: store the new brief artifact (as `open` does), emit `intent_revised { brief: ArtId, affects }`. The fold counts versions: `intent_version` is 1 at `task_opened` and +1 per `intent_revised`.
- For every affected claim (in `claims`, or with a domain in `domains`, or all):
  - `accepted`, not acted on → `claim_reopened { by: intent, epoch+1 }`, then the existing budget line and dependency cascade;
  - `accepted` and acted on (a mark exists in the current epoch) → `claim_escalated { reason: intent_changed }`;
  - `open`, `escalated` → nothing (still being judged; new evidence is judged against the new brief);
  - `dropped`, `void`, `superseded` → nothing (dropped stay dropped).
- Each claim's `proven_at`: the `intent_version` at its latest acceptance (fold only, no stored field), shown on `claim_card` with the task's current version. `claim_line` unchanged.
- `HostAction::TaskPaused { reason: PauseReason, detail: String }`, `PauseReason = SpendCeiling | CallCeiling | Breaker`. Needs the task `open`; emits `task_paused { reason, detail }`, which moves the task to `escalated`. Agent writes are then refused by the existing C-R2; the human resumes with the existing `resumed(open)`. Same op replay returns the same result (R15).
- New codes: `bad_affects`. New `ReopenCause::Intent`, `ClaimEscalation::IntentChanged`.
- Spec: addendum gains a "#598: intent versions and pauses" section, rules C-R11/C-R12, event table rows, schema list item 13, and trace tests CL-xx written before the engine change.
Host:
- `bbp human intent --brief FILE [--claim ID]... [--domain D]...` (no claim/domain = all); `bbp host pause --reason spend_ceiling|call_ceiling|breaker --detail TEXT` for orch.
- Inbox shows `task_paused` (reason, detail, resume command) and `intent_changed` escalations.

## PR 2 — host: brownfield (gap 1)
- `bbp open --base REF`: resolve REF to a full commit id and pin it as `ProfileSet.baseline`. Run the frozen build suite (the task's profile set, `suite: build`) on a clean checkout of base, in the same sandbox a candidate run uses.
  - Red: refuse the open `baseline_red`, listing each red profile (and failing test names when the report has them) and the one profile-set edit that moves them out (e.g. a skip arg for that profile). Nothing is written.
  - Green: write the canonical baseline report to `<dir>/baseline/<task>.json`; its SHA-256 goes in `open`/`task_opened.baseline_digest` (new optional field, not written when absent).
- Every candidate already re-runs the suite, so `failing(c)` blocks a break; nothing new there.
- `base_test_changed`: refuse storing a candidate whose diffs modify or delete a file that matches `test_paths` and exists at base (adds don't count), naming every path, unless the candidate's claim is `reversible: false` (which C-R6 then sends to Nano). The inbox item for that irreversible claim lists every such test, one item per candidate (Q2).
- Survey lens: an orch lens kind (config), posting behaviour claims; its characterisation tests go in through the existing `tests` claim path. Prompt text only; it runs under the #594 runner.

## PR 3 — sandbox and data boundaries (gap 2) plus the breaker (rest of gap 4)
Host:
- Named sandbox profiles per process kind in host config: `runner` (no endpoints, as today), `builder` (internet only, as `confine.rs` today), `lens` (internet only), `shadow:<name>` (no endpoints, read-only `real_data` roots, write only to its plan directory). Existing hardcoded modes become the defaults, so behaviour is unchanged without config.
- Filtered view: a per-turn copy of the tree that leaves out every path matching `sensitive_paths`, names included. Cloud lenses and cloud builders read only this view.
- `shadow:` kind: a read-only, no-network dry run over `real_data` mounts that writes one plan artifact. The plan is `sensitive` and stays host-side; only counts are `internal`. Real writes are an irreversible claim (Nano).
orch:
- Factory config: `domain_classes`, `sensitive_paths`, `real_data` (`{name: abs_path}`), and `breaker { window_calls: 40, window_usd: 5, min_score_move: 4, mode: observe|enforce }`, default `observe` (Q4).
- Item-class table in orch-store: each post's class = max(its domain's class, the highest class in the lens's context), so a lens that saw sensitive input taints what it posts (taint never goes down).
- `orch-lens::breaker` (pure): a window closes at 40 calls or $5, whichever first; it trips when no claim settled in the window and the sum of |Δscore| over the task's claims is under 4. Observe mode logs a would-trip; enforce mode returns a pause with `reason: breaker`. The #594 runner calls it and writes `bbp host pause`.
- `rusty_orch lens breaker [--task T] [--json]`: read-only state and would-trip report.
- Intent lens kind: suggests `affects` for an `intent_revised` (Nano still states it, Q3).

## Order and ownership
PR 1 first (protocol the others use). PR 2 and PR 3 are independent of each other. BBP owns: intent fold, pause event, refusals, baseline, sandbox profiles, filtered view, shadow kind. orch owns: classes and taint, prices, meter and ceilings, breaker, Survey and Intent lenses. The #594 thread wires the breaker and ceilings to `bbp host pause` in `lens run`; if `lens run` has merged by then, PR 3 wires it.

## Risks
- The design file's exact §6 shapes may differ from these defaults (Q16 in OPEN_QUESTIONS asks for it again; reconcile when it turns up).
- The breaker threshold (4) is unsourced; harmless while observe-only.
- Concurrent edits with the #594 thread in `rusty_orch/src/lens/*`.

## v1 changes after the Goldfish review ([GOLDFISH-REVIEW.md](GOLDFISH-REVIEW.md))
- B1: the baseline pin only runs with `protected` set. PR 2 checks `off_baseline` whenever `baseline` is set; builders detach at the baseline, not HEAD (S5).
- B2: "acted on" is the existing `acted_on()` (any mark not aborted, any epoch). Trace test for acted-in-epoch-1, re-accepted-in-epoch-2.
- B3: PR 1 adds `TaskPaused` and `IntentChanged` to Mill Board's inbox `Reason` (strict decode), with plans: pause → resume open; intent_changed → accept / drop.
- B4: CLI lists are comma-separated (`--claims msg:1,msg:2 --domains a,b`); args.rs keeps only the last repeated flag.
- S1: `task_paused` is followed by `transition(Escalated)`, so `resumed(open)` works; host inbox tracks the latest of `escalated` and `task_paused`.
- S2: a ceiling pause re-pauses after resume unless the ceiling changed; the inbox item says which ceiling to raise. No per-task allowance (would be new schema).
- S3: PR 2 touches rusty_bbp: `baseline_digest` on `OpenTask`/`task_opened`, codes `baseline_red`, `base_test_changed`, golden for no `--base`.
- S4: the filtered view is a plain copy with no `.git`; cloud diffs touching `sensitive_paths` are refused; leak test with `git cat-file`.
- S6: C-R10/C-R11: an answer refs the *current* brief; after `intent_revised` an old-brief Refine is `refs_required` (stale view, re-read).
- S7: `base_test_changed` is checked after `protected_path`; the inbox lists non-`add` `tests_changed` entries; item per candidate, Approve = accept the claim.
- S8: no breaker table; windows derive from meter calls and BBP revisions; would-trips go to the runner log.
- S9: affected claims are processed in ascending MsgId order, status read when reached; dependency cascades as today.
- S10: `--base` runs the baseline before anything is written.
- S11: the Intent lens is a host-side CLI suggestion (`rusty_orch lens intent`), posting nothing.
- Minor: `affects` defaults to `all`; unknown domains are accepted and match nothing; trace ids from CL-102; `bbp host pause` takes `--op`; the shadow command isn't `bbp shadow` (taken by #596); `domain_classes` is already delivered.

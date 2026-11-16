# #1124 Spend estimate, ceiling and provider checks (draft 2026-11-16)

Status: proposal for Nano; answered 2026-11-16, no paid runs for now (D134). Nothing is built and nothing here authorises a paid call (D38, ADR-0021, D130). No model was called to write it.

## 0. What "spend" means in v0
- **Nothing is metered in dollars.** `config.rs` refuses any nonzero `price` (ADR-0021: v0 runs flat-rate subscriptions and local models), so the meter charges 0 per call. `experiment_usd`, `task_usd` and the breaker's `window_usd` cannot trip. 598-build/PLAN.md: "money ceilings don't bind in v0; the call ceiling does".
- **Binding limits today:** `task_calls` (STARTER 400) stops a task; `--max-passes` (default 20) stops a run; budget units cap posts and candidate runs per claim (8, 16 or 32, doubled if irreversible; Q19).
- **The breaker is not wired into `lens run`** (#758), so `mode: enforce` does nothing yet.
- **Real money can leave only through a forbidden route:** an API key, credits or overage, a nonzero price, or a Codex login switch. ADR-0006 and ADR-0012 scrub API-key env vars. The `no_overage` assertion expires after 30 days.
- **Every dollar figure below is a shadow figure:** what the same tokens would cost at API list rates. It measures plan-quota use. It is not a bill.

## 1. Run model
Calls (`schedule.rs`, STARTER): a question gets 3 proposal calls (2 proposers + the rebel, which may retry up to 2). Each claim gets 3 reviewing lenses (research, adversary, operator), each making 1 blind call, 1 look-harder call if no counter was found, and 1 informed call per wave of new foreign evidence. Calls per claim: low 3, expected 8, high 12. Routine-tier calls spread roughly evenly over Claude, Codex and local; shadow dollars price only the Claude third.

Tokens per call: **no recorded lens transcripts exist**, so no measured figure. Measured bounds only: `MAX_OUTPUT_TOKENS` 4,096, context 200k (cloud) / 32,768 (local). Assumed in/out per call: low 6k/0.5k, expected 12k/1.5k, high 30k/4k. At the repo's Opus-class list rate ($4 in / $20 out per MTok, `docs/bbp/2b-criteria.json`): $0.034, $0.078, $0.20 per call, no cache discount.

Builder sessions (#600, Claude CLI only, D64): low 1, expected ~1.3, high up to 3 per candidate; 45 minutes each (D66 default); shadow $2 / $5 / $12 per session, **unmeasured**, loosely anchored on #596's ~$9-12 per paired run.

## 2. Low / expected / high (shadow $)
| Run | Claims | Lens calls | Claude lens $ | Builder sessions | Builder $ | Total |
|---|---|---|---|---|---|---|
| Smoke run, paid half | 1 toy (high 2) | 3 / 8 / 24 | 0.03 / 0.23 / 1.6 | 0 | 0 | **0.03 / 0.23 / 1.6** |
| 1 dedup stage-1 run | 4 / 7 / 15 | 15 / 59 / 185 | 0.17 / 1.6 / 12 | 3 / 4 / 9 | 6 / 20 / 108 | **~6 / ~22 / ~120** |
| 10 stage-1 runs | | 150 / 590 / 1,850 | | | | **~60 / ~220 / ~1,200** |

Stage-1 high (185 calls) fits inside STARTER's 400. The expected 10-run figure (~$220) exceeds #596's $200, which was a Phase B ceiling only (parked); it does not carry over. Builder sessions drive expected and high; they are the least known input. #596 prior: ~$90-120 for 10 paired lite runs (remind-me only); ceiling $200 total / $25 per run, pause if the first paired intent costs over $40 (D38).

## 3. Assumptions
Measured (repo): call, pass and reroll constants; `MAX_OUTPUT_TOKENS`; STARTER ceilings 200 / 25 / 400; breaker defaults 40 calls, $5, score move 4, observe; Opus 5.5 / Sonnet 5.5 list rates (owner-checked 2026-10-19).
Assumed: tokens per call; the 1/3 Claude share; look-harder and informed rates; claims per run; builder cost and count; no cache discount; "10 runs" = 10 stage-1 runs.
Fix: read the first smoke run's actual `Usage` from the log and replace these.

## 4. Provider terms
| Provider | Billing path in v0 | Check |
|---|---|---|
| Claude CLI | Subscription login only; API-key and cloud env vars scrubbed; no-overage asserted per account email (ADR-0001, ADR-0012, R7) | Plan caps, overage setting and terms are not in the repo (unverified); Nano reads them off the account page |
| Codex CLI | ChatGPT subscription login; API keys scrubbed because Codex once silently took the paid path; no-overage bound to machine id; a login switch on that machine is not detected (ADR-0006, R7) | A real rate limit was never triggered in development; plan caps and overage terms unverified |
| Local (llama-swap, BaileyAi) | $0 metered, own hardware (ADR-0020) | Model ids and port unverified (see 1123) |
| Claude API list rates | Not used in v0; shadow dollars only | Dated 2026-10-19; re-read before relying on them |

Public plan prices and caps are not stated in any repo doc and are not given here.

## 5. Ceiling options
| | A. Smoke only | B. Smoke + 1 stage-1 run | C. M2 (10 runs) |
|---|---|---|---|
| `task_calls` | 60 | 250 | 400 |
| `experiment_usd` (shadow) | 10 | 50 | 200 |
| `task_usd` (shadow) | 5 | 25 | 25 |
| breaker `window_calls` / `window_usd` / `min_score_move` | 20 / 1 / 4 | 40 / 5 / 4 | 40 / 5 / 4 |
| breaker `mode` | observe (enforce unwired) | observe, enforce once #758 wires it | same as B |
| First-run pause | report calls and quota use | pause if the stage-1 run uses over 150 calls or $40 shadow | same as B |
| Real dollars at risk | $0 | $0 | $0 |

The `*_usd` values are report thresholds only; only `task_calls` stops anything in v0. Recommended: **B**, then raise to C after the first run's actuals.

Smoke run paid half by option (expected / high / hard worst case, every call to Claude at $0.20): A ~$0.23 / $1.6 / $12; B same / same / $50; C same / same / $80. The local-only smoke run costs $0.

## 6. Reminder (dated)
Before 2026-12-03: re-check that Codex and Claude are on plans with no overage or credits enabled, then re-run Nano's `no_overage` assertion in `/etc/let-em-loose/assertions.json`. It lapses 30 days after its date, after which lens calls stop being admitted. Also confirm the Codex login on the BaileyAi machine has not been switched; the check cannot see that. (The 2026-12-03 date is from the card.)

## Open for Nano
**Answered 2026-11-16 (D134).** Nano chose "No paid runs for now" on the decision card (tap 2026-11-16T17:20:01Z, thread `cmsg_01632CBKCXsWe6RyFWvE9dovKYDoFmtvXi6TAYDxoS9Gmq`, card `cmsg_01632CBKCXsWe6RyFWvE9dovEdZwxaX7X8V2c9ew2bfAKe`), over A (smoke only), B (smoke + 1 stage-1 run) and C (full M2). No ceiling is authorised. Reading (the expert's): the paid half of #1166 is deferred and its first run is local-only (`1166-smoke-run.md`); the M2 paid experiment has no ceiling; §5 stays on file for when he revisits. D38's $200 / $25 / $40 still apply to Phase B only. Nonzero prices stay refused (ADR-0021). Follow-up: OPEN_QUESTIONS Q49.

## Drift
No lens transcripts exist, so the card's "tokens measured from recorded transcripts" cannot be met yet. The card's "paid" and "breaker settings that match it" do not fit v0: money ceilings cannot trip and the breaker is unwired (#758). #596's estimate lives only in remind-me. STARTER pins `model: "opus"` without a version. `builders_per_claim` is approved (D66) but its value is not in the repo.

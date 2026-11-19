---
category: Added
changelog: Let Em Loose smoke run 7 recorded (three candidates from three providers; the first lens-accepted survivor); the audit scope for accepted claims is Q62.
---
## 2026-11-18 - Let Em Loose smoke run 7 record

- **Added:** a "Run 7 result" in `docs/projects/let-em-loose/1166-smoke-run.md`: three candidates from three providers (criterion 2 met for the first time, with no empty Qwen replies), claim 6 accepted by the lenses at rev 72 (rule AcceptLine, two support authors, score 10; confirmed by Nano's `bbp why` output) on votes that quote the brief accurately, claim 8 dropped on two grounded votes, claim 7 left Open (the lenses caught its floor deviation), the A2 fixture the only escalation. In `handoff/OPEN_QUESTIONS.md`, the A2-citation evidence under Q53 (no vote cited msg 3 or msg 2 as a run; Q53 stays open, since one run is not enough) and Q62 (should the grounding audit also cover Accepted claims).
- **Added:** D164 in `DECISIONS.md` (Nano: smoke run 7 is the first run under D143, so the D127 design freeze lifts; it releases the #1402 builder-session design rebuild (D129) and #600 first; the packs design and the Survey and Intent lenses stay for later) and a "Lifted by D164" line on D127.
- **Known limitations:** docs only; the human steps are board-confirmed (claim 7 dropped by Choose on Q 1 at rev 118, A2 dropped by Nano at rev 121, inbox empty), so smoke run 7 is recorded as the first clean #1166 pass under D143; Q53, Q51, Q55, Q56 to Q58, Q60 and Q62 stay open.

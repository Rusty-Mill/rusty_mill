# Trace Catalog

Every failure sequence the four review rounds traced, as a numbered row with the test that reproduces it. Each test must fail against a core with the corresponding fix removed. Test names: `tests/traces.rs::tr_NN_*`.

| Id | Source | Sequence | Expected |
| --- | --- | --- | --- |
| TR-01 | r1 ChatGPT #1; r2 Claude F1 | Tester and Reviewer approve candidate A; Coder stores candidate B at `merge_gate` | B rejected `wrong_state` (gating), approval on A still valid |
| TR-02 | r1 ChatGPT #1 | Human approves merge naming a candidate that is not current | `stale_subject` |
| TR-03 | r2 Claude F2; ChatGPT F1 | Plan approval names a spec other than the one in the gate request | `stale_subject`; correct spec accepted |
| TR-04 | r2 ChatGPT F3; r4 ChatGPT F2 | Coder's turn ends; Coder is regranted; write with the previous turn's token | `stale_turn` |
| TR-05 | r2 Claude F4; ChatGPT F4 | Candidate stored; before the runner reports nobody holds a turn; after the report the Tester does | turn None, then Tester |
| TR-06 | r2 Claude F7; ChatGPT F8 | Candidate manifest lists a non-diff artifact | `wrong_kind` |
| TR-07 | r3 Claude F1; ChatGPT F1; r4 both F1 | Human `rerun`; old run's supervisor stores a report with the old secret | `stale_run`; new run selected; verdicts staled |
| TR-08 | r3 all; E2 | Human rejection to `planning` after a candidate exists; resume or verdict on the old candidate | candidate deselected; verdict `stale_candidate` |
| TR-09 | r3 Gemini F1; r4 F2/F4 | Coder in `build` yields to Tester; Tester posts a finding and passes | Coder regranted with a fresh token |
| TR-10 | r3 Claude F3; ChatGPT F8; E6 | Candidate submission ends the turn; response lost; same `op` retried with the dead token | original `Stored` returned, no new events |
| TR-11 | r3 Claude F4; ChatGPT F5 | Escalated from `test` with no Tester verdict; human resumes to `review` | `not_reviewed` |
| TR-12 | r3 Claude F8; ChatGPT F5; r4 Gemini F5 | Agent exhausts `reads_per_turn` | turn ends, regrant; task `reads` total still counts |
| TR-13 | r4 Claude F3 | Runner reports `error` | task `escalated` from `test`, no Tester turn |
| TR-14 | r4 Claude F2; ChatGPT F4 | Coder yields to Reviewer in `build` | `wrong_state` (not consultable) |
| TR-15 | r4 ChatGPT F6 | Human rejection with a stale `rev` | `stale_rev`; same `op` replay of an accepted rejection returns `Ok` with no effect |
| TR-16 | E3 | Message budget crossed | exactly one `escalated` for `messages`; crossing write accepted |
| TR-17 | E4 | Coder opens a request; state rebuilt from the log | no default turn granted while the request is open |
| TR-18 | E5 | Human answers an informational request | request settled; Coder regranted |
| TR-19 | A4 | Command arrives after the turn deadline but before any `Tick` | `token_expired`; turn already ended and regranted |
| TR-20 | r1 all; R7 | Agent posts a `decision` | `decision_forbidden` |
| TR-21 | r1 all | Reviewer reads messages | sees only human messages addressed to it |
| TR-22 | r1 Claude F4 | Coder stores a `test_report` | `kind_forbidden` |
| TR-23 | r4 ChatGPT F10 | Merge receipt names a different candidate | `stale_subject` |
| TR-24 | E1 | Human `rerun` | exactly one `RunStarted` after the rerun; entry action starts none |
| TR-25 | r3 Gemini F5 | Fragment over 128 chars | `fragment_too_long` |
| TR-26 | r3 Claude F7 | Yield while already in a consultation turn | `yield_nested` |
| TR-27 | r2 all | Spec stored without referencing the brief | `ref_unresolved` |
| TR-28 | r4 Claude F4 | Escalated from `approved` on budget; extend; resume to `approved` | allowed; merge receipt closes |
| TR-29 | A3 | Store conflict on append | driver reloads and recomputes with the same `op` |
| TR-30 | r2 ChatGPT F7 | Candidate submitted; tester verdicts; new candidate submitted | verdicts on the old candidate staled |
| TR-31 | model test counterexample (stage 1) | Tester `revise` returns to `build`; human rejection to `planning` | candidate deselected and its verdict records cleared |

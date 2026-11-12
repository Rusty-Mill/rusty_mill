# Decisions — millwright
Nano's decisions live in `../DECISIONS.md`: one row per decision, keyed, each with date,
reason and a remind-me id. Only `[stated]` answers go there; no agent defaults.

`[default]` choices `millwright-expert` makes (naming, file layout, which milestone code a
card is filed under when the roadmap already settles it) are logged below. Append-only;
supersede, never edit.

| # | Decision | Why | Rejected | Tag | Source |
|---|---|---|---|---|---|
| D1 | Defaults go here; `../DECISIONS.md` stays Nano-only | Nano: "DECISIONS.md holds only what Nano has stated … No agent defaults." | One mixed log | [default] | thread cmsg_01632CBKCXsWe6RyFWvE9dovLW5oB3NEACWHuvMnsptmYP, 2026-11-12 |
| D2 | Questions for Nano stay in `../QUESTIONS.md`; `OPEN_QUESTIONS.md` here only logs questions threads raise | One list for Nano | Copying QUESTIONS.md here | [default] | this pack |
| D3 | Phase tags on Mill Board are spelled `millwright:phase-0` … `millwright:phase-4`, always alongside the plain `millwright` tag | Board tags use `key:value`; bare `phase:N` is already taken by rusty_mux's own phases (28 cards), so an unprefixed tag would mix programmes | `phase-0` / `phase:0` (collides with rusty_mux), `mw-p0` (cryptic) | [default] | Mill Board tag census 2026-11-12; `../ROADMAP.md` § Putting this on the board item 5 |
| D4 | Triage lists name any open card claimed by another agent (Ledger #928 Planning, Conflux #847 Building, both claimed by `chatgpt`) with a line asking Nano to tell that agent to release or close it once the list is approved; the triaging agent never releases another agent's claim, with admin or otherwise | close-ledger and pause-conflux already count these cards (37 = 36 Backlog + #928; 30 = 29 Backlog + #847); only the claimant or Nano acts on a claim. That `chatgpt` is Dottius is unverified, so the list says "the `chatgpt` actor (Dottius?)" | Releasing with the admin credential; leaving the claimed cards off the list | [default] | thread 1 question, board facts 2026-11-12 |
| D5 | Swarm OS "relabel as a prototype" (archive-swarm-os) is proposed as both: a `prototype` tag on each of the 47 cards, listed in the triage list for Nano's approval, and a one-page conclusion in remind-me (R0). S0's Swarm OS part is both, since S0's "Done when" names the label separately from the conclusion | Tag is reversible and visible on the board; conclusion is R0/S0's other half. A tag does not change shipped counts in metrics; if Nano meant the metrics, that is a Mill Board change and goes back to him | Conclusion only (fails S0 "labelled a prototype"); tag only (fails R0) | [default] | `../ROADMAP.md` § Side projects S0, § remind-me R0; `../DECISIONS.md` archive-swarm-os |

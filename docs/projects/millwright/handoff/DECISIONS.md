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

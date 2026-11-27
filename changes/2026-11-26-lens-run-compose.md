---
category: Added
changelog: `rusty_orch lens run` composes each lens prompt per claim domain when the lens config sets `"compose": true` (#723).
---
## 2026-11-26 - lens run composes prompts per claim (#723 slice 3)

- **Added:** a lens config key `"compose"` (default false). When true, `lens run` opens the Factory catalog beside the config (`domains.json` and the files it names) and the `--repo` `DOMAIN.md` at start, and each job uses the prefix composed for its claim's domains, built on the same brief as the flat prefix. Composed sections are fitted against each provider's actual window: the card and the base prefix are never cut, the largest composed prefix that fits with the card is used (sections drop knowledge first, then DOMAIN.md, then standards), and a card that does not fit even with the base prefix is refused for that provider. Per domain set, orch keeps one unrendered `compose::Plan` (the sections in drop order and the prompt's byte length at each step) beside the lens's frame (brief, rules, reply protocol, kept once per lens), and renders only the prefix each provider's window selects. Every section left out, at the catalog's limits or to fit a window, is logged. When it is false or absent, behaviour and prefixes are unchanged.
- **Known limitations:** unknown claim domains are not logged. The Ai Env Manager manifest and the #1378 tool request are not read yet.

---
category: Added
changelog: `rusty_orch lens run` composes each lens prompt per claim domain when the lens config sets `"compose": true` (#723).
---
## 2026-11-26 - lens run composes prompts per claim (#723 slice 3)

- **Added:** a lens config key `"compose"` (default false). When true, `lens run` opens the Factory catalog beside the config (`domains.json` and the files it names) and the `--repo` `DOMAIN.md` at start, and each job uses the prefix composed for its claim's domains, one per distinct domain set and lens, built on the same brief as the flat prefix. Files left out to fit are logged. When it is false or absent, behaviour and prefixes are unchanged.
- **Known limitations:** unknown claim domains are not logged. The Ai Env Manager manifest and the #1378 tool request are not read yet.

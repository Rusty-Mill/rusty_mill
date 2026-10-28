---
category: Added
changelog: Mill Board admins can turn card movement rules off and on from Settings > Rules; the choice is kept in rules.json.
---
## 2026-10-28 - Mill Board: turn card movement rules off and on

- **Added:** a Settings > Rules tab (`settings_rules_set`, admin only) that turns card movement rules off and
  on. Off, any card may move to any column (Ready to Done, back to Backlog, out of Review) by anyone signed
  in holding a capability, with no claim needed; on (the default), moves follow the column table, each move's
  capability and the claim rule as before. The choice is written to `rules.json` in the settings directory and
  survives restarts. Each change, and each move made while the rules are off, is logged.
- **Known limitations:** Done stays terminal either way. Sign-in and project limits still apply. `card_take`
  still follows the normal queues. The rules file has no flag of its own; it lives only in `--settings-dir`.

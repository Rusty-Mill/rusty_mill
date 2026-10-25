---
category: Added
changelog: rusty-mux-ctl, a scriptable rusty_mux client generated from the request catalog, with --json output and stable exit codes.
---
## 2026-10-25 - rusty-mux-ctl

- **Added:** `crates/apps/rusty_mux/crates/ctl` (`rusty_mux_ctl`, binary `rusty-mux-ctl`).
  - Every method in `rusty_mux_proto::schema::CATALOG` is a subcommand and each parameter a `--flag` (`--flag value` or `--flag=value`). Arguments are checked before anything is sent.
  - Requests joined by `+` run in order on one connection and stop at the first failure, so `attach … + acquire-lease … + input …` types into a block.
  - `events [--after EPOCH:SEQ] [--follow]` prints a cursor line, the events since the cursor, and, without `--follow`, a closing cursor line to resume from.
  - `--json` prints one object per line. `--socket` or `$RUSTY_MUX_SOCKET` names the server.
  - Exit codes: 0 ok; 1 refused, or a cursor that can't be resumed; 2 usage; 3 no server or a broken connection; 4 a protocol error. An interrupt ends the process by the signal, which a shell reports as 130.
  - `tests/ctl.rs` covers every code against a real server.
- **Added:** `schema::Returns::Unanswered`, now used by `input`, which gets a reply only when it fails. Generators confirm such a request by following it with an answered one. `Method` now implements `Eq`, comparing by name.
- **Known limitations:**
  - A request's results can't feed the next request in a chain.
  - `--data` is the argument's UTF-8 bytes, so use the shell's `$'…'` for control characters.
  - Unix only. The client still negotiates `CHECKPOINT_V3`, so `attach` works, but it ignores the screen it receives.

# ADR-0004: `rp-cli setup` -- static config-file rewriting for third-party CLI tools

Status: Accepted
Date: 2026-08-07

## Context

`gap-analysis.md`'s OmniRoute comparison explicitly declined one of
OmniRoute's differentiators: `omniroute setup-*`, which points third-party
coding CLIs at itself via a **MITM proxy** -- a local TLS-intercepting
proxy plus a trusted CA installed into the target tool's trust store.
`ARCHITECTURE.md`'s non-goals list that verbatim: "no MITM-based
third-party CLI config injection."

An operator asked for a narrower, non-MITM version of the same outcome:
several popular OpenAI-wire-compatible coding CLIs (opencode, Crush, and
others) already read their provider/endpoint settings from a local JSON or
TOML file on disk. Pointing one of them at rusty_provider doesn't need
traffic interception at all -- it needs one field in that file rewritten.
That's a materially different, much smaller-blast-radius operation than a
MITM proxy: no TLS interception, no trust-store changes, no process
sitting in the middle of unrelated traffic, and (unlike a MITM proxy) nothing
that keeps running after the command exits.

## Decision

Add `rp-cli setup` (`list` / `show` / `apply`), a **static file-patcher**
scoped to exactly one job: given a known third-party tool's config file,
rewrite only its endpoint field(s) to point at a running rusty_provider
instance. It never starts a proxy, never touches TLS/certificate state,
and never runs after the command returns.

Key design constraints, chosen to keep this in the same
"stays honest, reversible, no help pretending" register as the rest of
`rp-cli`:

- **The set of known tools is data, not code.** `crates/cli/cli_targets.toml`
  is a plain TOML file (default set baked into the binary via
  `include_str!`, exactly like `config.example.toml` is the template for an
  operator's own `config.toml`) listing each tool's config path, file
  format, and the dotted field paths to set. Adding or fixing a target --
  including an operator's own private/internal tool -- is a data edit via
  `--targets <path>`, never a new Rust match arm. Third-party config
  schemas also drift with tool versions; keeping this data-driven means a
  broken path can be patched without a rebuild.
- **Only the endpoint gets rewritten -- never a secret.** `rp-cli`'s
  existing rule ("never prints a resolved secret's value") extends here to
  "never *writes* one either." When a target's field spec calls for an API
  key, `rp-cli` writes that tool's own env-var-reference syntax (opencode's
  `{env:VAR}`, Crush's `$VAR`) naming whatever env var the operator passed
  via `--api-key-env` -- never a literal token, and `rp-cli` never reads
  the variable's value to do this.
- **`show` before `apply`, always a backup, always additive.** `show`
  (the default a new operator reaches for first) is a pure dry run --
  prints the file that would be written, touches nothing. `apply` requires
  an explicit `--yes` and copies the original file to `<path>.bak` before
  writing (skipped only when the file didn't already exist). The patcher
  merges into whatever's already in the file -- unrelated keys survive --
  and refuses (rather than clobbers) if an existing non-object value sits
  where a field's path needs an object.
- **Still no network call.** Resolving `~` in a config path and writing a
  local file doesn't cost this crate its "synchronous, read-only-except-
  when-you-say-so, no async runtime" character -- `setup apply` is the one
  command in the crate that writes anything, and it writes only the file
  it was pointed at.

`ARCHITECTURE.md`'s non-goal bullet is narrowed from "no MITM-based
third-party CLI config injection" to "no traffic interception" --
static, opt-in, data-driven config-file rewriting for tools that already
support pointing at a custom OpenAI-compatible endpoint is now in scope;
a MITM proxy or trust-store changes are not, and would need their own ADR.

## Alternatives considered

- **MITM proxy (OmniRoute's actual approach).** Rejected again, for the
  same reason `gap-analysis.md` gave the first time: a local TLS-
  intercepting proxy plus CA trust-store changes is a much larger,
  harder-to-reverse, harder-to-audit blast radius than this crate's
  existing "never makes a network call" design center, for a problem this
  narrower approach already solves for every tool that reads its config
  from a file.
- **Hardcode the tool list in Rust (one match arm per tool).** Rejected:
  third-party config schemas are outside this repo's control and drift
  independently of `rp-cli` releases (see e.g. Codex CLI dropping
  `wire_api = "chat"` support entirely in favor of `wire_api =
  "responses"` -- a tool that was a valid target yesterday can stop being
  one without any change on this end). A data file an operator can patch
  or extend without waiting on a new `rp-cli` release is strictly more
  useful, and costs nothing extra to build since the field-path patcher
  has to be generic either way.
- **Write literal API keys into the target config.** Rejected: breaks the
  "never handle a resolved secret" rule the rest of `rp-cli` already
  follows, for no real gain -- every target tool checked so far supports
  an env-var-reference syntax in its own config format, so there's no
  case where a literal value is actually necessary.

## Consequences

- `rp-cli` gains its first command that writes to disk. Every other
  command in the crate stays read-only; `setup apply` is scoped tightly
  (one file, one backup, `--yes`-gated) so this doesn't erode the "safe by
  default" character the rest of the crate relies on.
- The shipped default target list (`opencode`, `Crush`) is intentionally
  small and verified against each tool's current documented config schema
  rather than chasing broad coverage -- same "curated, not exhaustive"
  choice `docs/PROVIDERS.md` made for provider presets. Operators extend it
  via `--targets` without needing a new `rp-cli` release.
- Because the target list is versioned data, a tool that changes its
  config schema (or drops file-based config entirely, as Codex CLI's
  `wire_api` history shows can happen) degrades to a stale `cli_targets.toml`
  entry, not a runtime crash -- `setup show` surfaces exactly what would be
  written, so a stale field path is visible before it's applied, not
  discovered by opening the target tool afterward.

# Deploying rusty_fedora_agent

Everything in this directory is a **reviewable template**. Nothing here
is applied automatically by this crate, by CI, or by any tool in this
repository -- a human reviews and applies each file to the target host
(e.g. baileyai) by hand. This is deliberate: privilege scoping (who can
run what, as whom, without a password) is exactly the kind of
hard-to-reverse, security-relevant change that should never happen
silently.

## Files

| File | Installs to | Purpose |
|---|---|---|
| `allowlist.toml` | `/etc/rusty-fedora-agent/allowlist.toml` | Units/packages/config-path prefixes this agent may act on. **Ships empty.** |
| `rusty-fedora-agent.service` | `/etc/systemd/system/rusty-fedora-agent.service` | Runs the agent as an unprivileged system user. |
| `polkit/50-rusty-fedora-agent.rules` | `/etc/polkit-1/rules.d/50-rusty-fedora-agent.rules` | Grants `systemctl start/stop/restart/enable/disable` for exactly the units listed, and no others. |
| `sudoers.d/rusty-fedora-agent` | `/etc/sudoers.d/rusty-fedora-agent` | `NOPASSWD` for `dnf install`/`dnf remove` only -- package-name scoping happens inside the agent, not here. |

## Order of operations

1. Build the binary (`cargo build --release -p rusty_fedora_agent`) and
   copy it to the target host, e.g. `/usr/local/bin/rusty_fedora_agent`.
2. Create the unprivileged system user and add it to `systemd-journal`
   (command is in `rusty-fedora-agent.service`'s own header comment).
3. Edit `allowlist.toml` to add exactly the units/packages/config paths
   you want this agent to manage, then copy it to
   `/etc/rusty-fedora-agent/allowlist.toml`.
4. Edit `polkit/50-rusty-fedora-agent.rules`'s `ALLOWED_UNITS` to match
   `allowlist.toml`'s `units` list, then install it and
   `systemctl restart polkit`.
5. Validate and install `sudoers.d/rusty-fedora-agent` with `visudo -cf`
   first (see its own header comment).
6. Install and enable `rusty-fedora-agent.service`, with `--bind` set to
   a private/Tailscale address -- never `0.0.0.0`.
7. Point `rusty_homelab_mcp` at it: `--fedora-agent-url
   http://<that address>:8765` (or the matching `FEDORA_AGENT_URL`
   environment variable).

## Keeping the two allowlists in sync

`allowlist.toml`'s `units` list and the polkit rule's `ALLOWED_UNITS` are
two independent enforcement points that must agree by hand -- this
agent's own allowlist check is not a substitute for the polkit rule (an
attacker who somehow bypassed this agent's own code would still be
stopped by polkit), and the polkit rule alone would let a mis-scoped
agent binary still fail safely. There is no single source of truth
between the two here in v1; if that drift becomes a real problem, teach
the agent to read `ALLOWED_UNITS` out of the same TOML file at startup
and generate the polkit rule from it, rather than duplicating it by hand
-- not done speculatively here since there's no second consumer yet.

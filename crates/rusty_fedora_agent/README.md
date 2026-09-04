# rusty_fedora_agent

An unprivileged local agent for scoped systemd/dnf/config-file control on a
Fedora Server host, exposed over a small local HTTP API.
[`rusty_homelab_mcp`](../rusty_homelab_mcp)'s `fedora` module (via the
[`rusty_fedora`](../rusty_fedora) client crate) is the intended, and only
supported, caller.

## Why this exists

`rusty_homelab_mcp` talks to OPNsense and Proxmox as typed REST clients --
no local exec, no SSH. Fedora has no equivalent management API, so this
agent runs *on* the Fedora host itself (built on
[`rustils`](../rustils)' portable process-spawning layer), and
`rusty_homelab_mcp` talks to *it* the same typed-REST-client way it talks
to OPNsense/Proxmox. That keeps `rusty_homelab_mcp` uniform -- one calling
convention across every backend, not REST for two and SSH+exec for a
third -- and keeps privileged execution logic on the box it's local to.

## Privilege model

This agent must never run as root. Each capability is scoped narrowly:

- **Journal reads** -- run the agent's own user in the `systemd-journal`
  group. No elevation needed.
- **Service control** -- a polkit rule grants `start`/`stop`/`restart`/
  `enable`/`disable` only for unit names in the allowlist config, not
  blanket unit control. See `deploy/polkit/`.
- **dnf install/remove** -- a `sudoers` `NOPASSWD` entry scoped to exactly
  `/usr/bin/dnf install`/`/usr/bin/dnf remove`, plus a package-name
  allowlist enforced *in this agent* before the command is ever built --
  an illegal package name never reaches `exec`. See `deploy/sudoers.d/`.
- **Config read/write** -- a path-prefix allowlist; writes take an
  automatic `.bak` copy of the previous content first.

All three allowlists (units, packages, config-path prefixes) live in one
TOML config file this agent loads at startup -- see
`deploy/allowlist.toml` for the format. It ships empty: nothing is
permitted until a human deliberately adds entries.

**The `deploy/` directory is reviewable templates, not something this
crate applies automatically.** Apply them to the target host by hand --
see `deploy/README.md`.

## Running

```sh
rusty_fedora_agent \
  --bind 100.x.y.z:8765 \
  --allowlist /etc/rusty-fedora-agent/allowlist.toml
```

`--bind` **must** be a private/Tailscale address, never `0.0.0.0` -- this
agent has no authentication of its own; network reachability is the only
access control it has, so it must never be exposed beyond the private
network `rusty_homelab_mcp` runs on.

## HTTP API

All responses are JSON. Errors are `{"error": "..."}` with a `4xx` status
for allowlist rejections and malformed requests, `5xx` for everything
else (a failed subprocess, a platform error).

| Method & path | Body | Response |
|---|---|---|
| `GET /status` | -- | `SystemStatus`: uptime, load average, memory, kernel/OS release |
| `GET /services?unit_type=service\|timer\|socket` | -- | `[ServiceSummary]`: name, load_state, active_state, sub_state |
| `POST /services/{name}/control` | `{"action":"start\|stop\|restart\|enable\|disable"}` | `{}` |
| `GET /journal?unit=&lines=&since=&priority=` | -- | `[{"line": "..."}]` (`journalctl -o short-iso` lines) |
| `GET /dnf/updates` | -- | `[PackageUpdate]` |
| `POST /dnf/install` | `{"packages": ["..."]}` | `{"task_id": "..."}` |
| `POST /dnf/remove` | `{"packages": ["..."]}` | `{"task_id": "..."}` |
| `GET /tasks/{id}` | -- | `TaskStatus`: state (`running`/`succeeded`/`failed`), stdout/stderr/exit_code once finished |
| `GET /config?path=...` | -- | `{"content": "..."}` |
| `PUT /config` | `{"path":..., "content":..., "backup": true}` | `{}` |

`fedora_dnf_install`/`fedora_dnf_remove` return immediately with a task id
-- installs can run long. Poll `GET /tasks/{id}` until `state` is no
longer `running`.

## Architecture

Ports-and-adapters: [`ports::SystemController`]/[`ports::PackageController`]
are the domain boundary; [`systemd::SystemdAdapter`]/[`dnf::DnfController`]
are the real, `rustils`-backed implementations, and
[`http::AgentState`] is generic over both traits so the HTTP layer's own
routing can be tested against a mock without a real Fedora box or a real
`Spawner`. [`allowlist::Allowlist`] is the scope check every mutating
adapter method calls before building a `Command` or touching a path.

## Not included (deliberately)

No named consumer yet, so not built speculatively: network interface
configuration, firewall rules on this host (OPNsense already owns network
policy), user/group management, or any form of arbitrary command
execution. Add a tool for one of these when something actually needs it.

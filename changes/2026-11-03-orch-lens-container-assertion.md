---
category: Added
changelog: The lens operator's host-cleared assertion can name the dedicated container a provider runs in, with its exact mounts; it fails closed until the container check lands.
---
## 2026-11-03 - rusty_orch: host-cleared container assertion

- **Added:** a `host_cleared` entry may name a `container` and its exact `mounts` (Let Em Loose #594, deployment approved by Nano 2026-11-03: Codex runs only in `lel-codex` via `podman exec`). Bind mounts give an absolute `source`, a `destination` and `rw`; volumes give `volume` and `destination`. The list must include a read-only bind of `/etc/machine-id`. A container clearance holds on any host, because the container is checked before each call. A Codex no-overage entry may be bound to the container's machine id.
- **Changed:** `Assertions::host_cleared` is `Option<Clearance>` (`Host` or `Container(ContainerSpec)`) instead of a bool. The new types are in `rusty_orch::lens::container`.
- **Known limitations:** a container clearance is parsed but not yet honoured, so Codex stays `Unavailable` under it. The container check and `podman exec` come in the next slice.

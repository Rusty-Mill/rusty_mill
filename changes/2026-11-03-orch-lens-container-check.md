---
category: Added
changelog: A host-cleared Codex lens provider can run only inside the operator's container, checked from the host before every call and reached by `podman exec`.
---
## 2026-11-03 - rusty_orch: lens container check and podman exec

- **Added:** under a container clearance (Let Em Loose #594, deployment approved by Nano 2026-11-03), every Codex lens question first runs `/usr/bin/podman container inspect -- <name>` as the runner's own user. The question proceeds only when all of these hold:
  - the container is running
  - it is not privileged and has no added devices or capabilities
  - its environment sets none of `OPENAI_API_KEY`, `CODEX_API_KEY` or `OPENAI_BASE_URL`
  - its mounts are exactly the asserted list (bind or named volume only)
  - the source of its read-only `/etc/machine-id` bind is a file the runner cannot change (the same ownership rule as the assertions file) and holds the asserted id

  Any failure is `Unavailable` and runs nothing in the container. The login probe and `codex exec` then run as `podman exec -i --workdir <dir> <checked id> …`, so a container recreated after the check is not used. `<dir>` is where the runner's repository is bind-mounted. A container that does not mount the repository gets no grant.
- **Added:** `assertions::guarded_read` (the ownership-checked read, now shared by `load` and the machine-id read), `container::{check, Contained, InContainer, Verified, read_machine_id}`.
- **Known limitations:**
  - Mount sources are compared as podman reports them, so the operator should name real paths, not symlinks.
  - Whether Codex's read-only sandbox works inside an unprivileged rootless container has not been tried, because no live call is authorized.

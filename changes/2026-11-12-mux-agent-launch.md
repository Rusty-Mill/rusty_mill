---
category: Added
changelog: rusty_mux can launch an agent with an exact environment and descriptors it passes at 3, 4, and on.
---
## 2026-11-12 - rusty_mux agent launch

- **Added:** C75 (card #749), behind `Features::AGENT_LAUNCH` (bit 42).
  - `Launch.env` (field 58) gives the program exactly that environment: nothing is inherited and there is no `RUSTY_MUX_*`.
  - `Launch.fds` (field 59) passes up to 8 descriptors with the launch frame. The program gets them at 3, 4, …. This works only on Linux, over a direct Unix connection.
  - A mismatch is refused as `bad_launch(fds)`, and the descriptors are closed.
- **Added:** `rusty-mux-ctl own` takes `"env"` and `"fds"` on a launch, and passes only descriptors it inherited.
- **Added:** rustils `platform-linux` has `LinuxPty::spawn_passing`, which maps passed descriptors without collisions. It needs glibc 2.34 or newer.

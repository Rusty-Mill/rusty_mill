---
category: Added
changelog: replay-viewer plays a physics capture with the port's simulation of it as a ghost car (ADR-0085, slice b5).
---
## 2026-10-24 - Rocket League: sim runs in the replay viewer

- **Added:** `replay-viewer <capture.jsonl> [--sim <candidate.jsonl>]` builds the 3D page from an ADR-0005 capture instead of a replay; with `--sim`, the port's candidate run is drawn as a pale, translucent ghost over the recorded car on the same timeline. New `replay_viewer::physics` module and an optional `ghost` flag on `ScenePlayer`, omitted from replay scenes.
- **Added:** `rb-verify --self <capture> --write-candidate <out.jsonl>` saves the simulation it scores in the capture format (`RB-VERIFY-003-FR-019`), through the new `rb_capture_ingest::write_frames`, which round-trips exactly through `CaptureFileSource`.
- **Changed:** the viewer's headless-GL CI job also renders `human/clean-dodge` with its sim run.
- **Known limitations:** no ghost ball yet; velocity arrows, the input strip (b6) and contact points (b7) come later.

---
category: Changed
changelog: rusty_term's GPU renderer keeps its instance buffers and draw lists between frames instead of allocating new ones each frame.
---
## 2026-10-26 - rusty_term: persistent GPU instance buffers

- **Changed:** the GPU renderer used to create three vertex buffers (base, overlay, image instances) every frame with `create_buffer_init`. Each one now lives on the renderer and is rewritten in place with `write_buffer`. It is reallocated only when a frame outgrows it, to the next power of two and at least 4 KiB, and a draw uses only the bytes that frame wrote. Layers with no instances skip their draw call.
- **Changed:** the per-frame base, overlay and image lists are cleared and refilled, keeping their capacity, so a steady frame allocates no new `Vec`s for them.
- **Tested:** a unit test for the growth rule. An ignored GPU test renders a small frame, a 60x40 frame that regrows the buffers, then the small frame again. Each frame reads back the same pixels as a fresh renderer, so instances left past the end from the bigger frame are never drawn. It runs under `--ignored`, as the existing render test does, and passes here on Mesa lavapipe. The existing single-cell render test now shares its readback helper.
- **Not measured:** on lavapipe a 120x40 frame takes 9–10 ms either way, which is software rasterization. The saving is three buffer allocations per frame on a real driver, and it has not been measured on hardware.
- Backlog row 11's other half, which draws the background as a flat per-cell color buffer, is not part of this change.

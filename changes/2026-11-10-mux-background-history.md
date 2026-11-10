---
category: Added
changelog: rusty-mux attach shows the screen first and streams older history after it, newest first, for the GUI.
---
## 2026-11-10 - rusty_mux: newest-first background history (C03 part 2)

- **Added:** `Features::BACKGROUND_HISTORY` (bit 41) and `Message::History` (0x90). With the bit, an attach's checkpoint carries the screen only. The older history follows Ready in batches, newest first, and live output is always sent before them. Detaching or attaching again drops history not yet sent.
- **Added:** rusty-mux-gui block connections ask for the bit and add each batch above the pane's history, rewrapped to its width and recoloured to its theme. The TUI and ctl are unchanged.
- **Added:** `rusty_mux_proto::checkpoint::HistoryReceiver` assembles batches from chunks.

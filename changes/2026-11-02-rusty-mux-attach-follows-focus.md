---
category: Added
changelog: The rusty-mux attach client follows each window's shared focus and the session's active window, and edits layouts with Ctrl-] H J K L, { }, and D.
---
## 2026-11-02 - rusty_mux attach client follows the focus (C41, part 3)

- **Added:** the attach client follows the window's shared focus: another client's `focus-block` or `move-focus` moves where the person types (and the input lease), and the client's own Ctrl-] focus keys move the shared focus. It switches to the session's active window when that changes. Attaching to a block focuses it.
- **Added:** Ctrl-] `H` `J` `K` `L` make the focused block narrower, taller, shorter or wider by 5 points of its split; `{` and `}` swap it with the previous or next block; `D` detaches it from the layout, still running. A server without `LAYOUT_EDIT` rings the bell.
- **Changed:** a window whose blocks are all detached stays open and blank until one is moved back, instead of ending the client; the client still ends when the window closes or its last block does.

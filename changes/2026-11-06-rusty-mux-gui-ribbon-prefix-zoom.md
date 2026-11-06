---
category: Added
changelog: rusty-mux-gui gains the ribbon, Ctrl-] prefix keys with numbered blocks, and zoom with a rail of the other blocks.
---
## 2026-11-06 - rusty-mux-gui: ribbon, prefix keys and zoom

- **Added:** arc 2 slice S3c (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D49, handoff D104). An attached window's ribbon runs along the bottom: what it is connected to, how many blocks it shows, the focused block's shell and the encoding, and the prefix keys. It replaces the status bar in attached mode only.
- **Added:** Ctrl-] arms the prefix as in the TUI: `|` and `-` split the focused block, `z` zooms or unzooms, `x` closes it, `h j k l` and the arrows move the focus, `1`-`9` jump to a numbered block, `d` detaches the window (the session keeps running), and Ctrl-] twice types it. While armed the ribbon lists every key and each block shows its number. The mode ends on the next key, Esc, a click or losing focus. The window's own split, zoom, close and focus keys send the same requests.
- **Added:** a zoomed block keeps its frame and header, with a "Zoomed" tag and "Exit zoom"; the other blocks wait on a rail at the right, each showing its status glyph and number, and a click focuses it. Every header has a zoom button.
- **Changed:** rusty_term's session-source port gains `Source::act` (split, zoom, unzoom, close) and `Remote::zoom`. Both renderers draw small pixmaps over the panes for the badges.
- **Known limitations:** an unknown key after the prefix is dropped without a sound: the GUI's bell only asks for attention while the window is unfocused, and the prefix needs focus. The ribbon's disconnected and reconnecting dots come with S4.

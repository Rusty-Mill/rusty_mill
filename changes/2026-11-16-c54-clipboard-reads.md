---
category: Added
changelog: A mux program's OSC 52 clipboard read is asked of the block's input-lease holder, which answers by its own policy; the server never answers it (C54).
---
## 2026-11-16 - Clipboard reads through the mux, gated by the client (C54, slice 1)

- **Added:** `Features::CLIPBOARD` (bit 48). When a program in a block reads the clipboard (OSC 52 `?`), the server sends `ClipboardRead{block, selection}` (0x91) to the block's input-lease holder only. The holder answers with `ClipboardReply{req, block, selection, data}` (0x2a), or `data: None` to deny; the server writes the OSC 52 answer to the program as that client's input. A reply from any other client is refused, as is one over 1 MiB, one for a read nobody asked, a second one, and one more than 30 s late. A program looping on the query asks the holder once.
- **Security:** reads are denied unless the client's own policy allows them (Nano, D309); the server never answers a read itself. No clipboard text goes in an event or the event history. A holder without the feature, or behind on its output, is not asked and the program gets no answer.
- **Public API (additive):** the `Features::CLIPBOARD` bit and the two messages above, behind the negotiated bit; older peers never see them.
- **Known limitations:** the GUI and TUI do not use the messages yet, so a mux GUI client still answers from its own grid under its `clipboard` policy (D312). Server-side selection and a script or `ctl` view of the clipboard are not built (Q80-Q82).

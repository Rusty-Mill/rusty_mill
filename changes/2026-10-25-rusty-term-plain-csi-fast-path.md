---
category: Changed
changelog: rusty_term parses plain CSI sequences in place, without allocating for up to 16 positional parameters, cutting instructions per byte by 38% on cursor_thrash and 27% on sgr_churn.
---
## 2026-10-25 - rusty_term: plain-CSI fast path

- **Changed:** CSI parameters parse into an inline array of 16 (`Params`) instead of a heap `Vec` built from `str::split`. More than 16 fields spill to the heap, so none are dropped. Each field keeps `str::parse::<usize>` semantics: empty, non-digit or overflowing is absent and takes the command's default.
- **Changed:** a plain CSI (`ESC [`, digits and `;`, a final byte) that sits wholly inside one `advance` chunk is parsed and dispatched straight from the input. It skips the byte-wise state machine and the copy into `param_buffer`. Private markers, `:` sub-parameters, intermediates, C0 controls inside the sequence, sequences split across chunks and parameters over 256 bytes still take the byte-wise path.
- **Changed:** SGR parameters parse from bytes with the same rules as before.
- **Changed:** instructions per byte: cursor_thrash 87.5 to 54.6, sgr_churn 110.5 to 80.4, alt_screen_flicker 75.8 to 69.9; ascii, scroll and unicode do not move. Ceilings in `bench/ir_ceilings.txt` drop to 60, 88 and 77. On the 4 MB cursor_thrash workload, wall-clock went from 91-105 to 159-170 MB/s on this VM, against 104-108 MB/s for libghostty-vt in the same runs.
- **Tested:** 28 CSI inputs give identical checkpoint state, responses and host output whether fed as one chunk (fast path) or one byte per call (byte-wise path). Those inputs include defaults, overflow, 16 and 20 fields, SGR colon forms, private modes, DECSCUSR, C0 and CAN inside a CSI, unterminated CSIs, and 256 and 257 parameter bytes. A mixed stream split at every byte offset matches the one-chunk result. A 17-field mode list keeps its last field.
- **Known limitations:** the fast path covers only sequences that arrive whole in one chunk. SGR still builds its own parameter list on the heap, and a CSI with more than 16 positional parameters spills to the heap. The checkpoint format is unchanged.

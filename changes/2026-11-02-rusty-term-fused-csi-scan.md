---
category: Changed
changelog: rusty_term scans and parses a CSI in one pass and takes private-marker CSIs on the fast path, cutting instructions per byte by 22% on sgr_churn, 11% on cursor_thrash and 8% on alt_screen_flicker.
---
## 2026-11-02 - rusty_term: fused CSI scan and parse

- **Changed:** the plain-CSI fast path parses each parameter as its digits go by, so the parameters are ready when the final byte is found. The scan and the separate split-and-parse pass are now one pass.
- **Changed:** CSI parameters live inline as plain values with a presence bit each instead of `Option<usize>` slots, halving the inline array to 128 bytes. A colon-free SGR reads its codes straight from it instead of building a heap list. Colon sub-parameter forms and lists of more than 16 fields still parse as before.
- **Changed:** a CSI led by one private marker (`?`, `>`, `<`, `=`), such as `CSI ? 1049 h` or `CSI > 1 u`, also takes the fast path when it sits wholly inside one chunk. A second or late marker, `:`, an intermediate or a C0 control still goes byte-wise.
- **Changed:** instructions per byte: sgr_churn 84.7 to 66.0, cursor_thrash 59.4 to 53.1, alt_screen_flicker 53.1 to 49.1; the other workloads do not move. Ceilings in `bench/ir_ceilings.txt` drop to 73, 58 and 54. On the 4 MB workloads (median of 3 runs of 20 iterations, noisy shared VM): cursor_thrash 182 to 228 MB/s, sgr_churn 110 to 162 MB/s, alt_screen_flicker 187 to 349 MB/s.
- **Tested:** 20 private-marker inputs give identical checkpoint state, responses and host output fed as one chunk or one byte per call. They cover alt screen, DA2/DA3/XTVERSION, Kitty keyboard and modifyOtherKeys relays, private DSR, XTSMGRAPHICS, DECSED/DECSEL, DECRQM, malformed markers, C0 and CAN inside, `usize::MAX` and one past it, 256 and 257 parameter bytes, a 17-field mode list and an 18-field SGR. A mixed private stream split at every byte offset matches the one-chunk result.
- **Known limitations:** sequences split across chunks still take the byte-wise path and copy into `param_buffer`. The checkpoint format is unchanged.

---
category: Changed
changelog: rusty_term_core RTCP v3 checkpoint encode is about twice as fast, with byte-identical output.
---
## 2026-10-27 - Faster RTCP v3 checkpoint encode

- **Changed:** `checkpoint::encode` checks cell style ids once per line run as it writes, instead of in a separate pass over every cell, and looks up a run's wire style by grid style id instead of hashing the resolved style. A 200x50 grid with 10,000 history lines encodes in 2.8 ms instead of 5.4 ms (46% fewer instructions). The output bytes are unchanged; decode still checks every restored cell.
- **Known limitations:** the writer keeps a 16-byte cache entry per grid style while encoding, so a grid with millions of distinct styles allocates tens of MB more during an encode.

---
category: Changed
changelog: rusty_term parses large OSC, DCS and APC payloads, such as inline images, about 7–9x faster.
---
## 2026-10-26 - Bulk scan of escape-string bodies

The parser used to collect the body of an OSC, DCS, APC, SOS or PM string one byte at a time, and an OSC re-checked its prefix-dependent size cap on every byte. On the first body byte it now finds the next terminator a word at a time (BEL or ESC for OSC; ESC, CAN or SUB for the rest), with the same std-only SWAR trick as the printable-ASCII fast path. It then copies the run into the buffer within the same caps. Over a 3.6 MB body fed in 4 KiB chunks (the ignored `measure_string_body_scan` test), an unknown APC goes from 432 to about 3,200–4,100 MB/s, and an unknown DCS from 344 to about 2,600–3,200 MB/s. OSC 52 stays at about 40 MB/s, because decoding and forwarding the clipboard payload dominates. Instructions per byte on the CI throughput workloads change by under 1%. New tests cover the scan against a byte loop and every two-chunk split of a mixed string stream, and check that the 4 KiB OSC cap holds at any chunk size.

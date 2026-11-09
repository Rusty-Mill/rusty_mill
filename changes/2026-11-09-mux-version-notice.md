---
category: Added
changelog: rusty_mux: clients name the features an older server lacks (C36 slice a); Hello and Welcome carry each side's version.
---
## 2026-11-09 - rusty_mux version notice

- **Added:** Hello and Welcome carry the sender's `rusty_mux_proto` version (field 57). Older peers skip it.
- **Added:** when the server lacks features the client asked for, `rusty-mux-gui` shows an info banner naming them and the server's version, `rusty-mux` prints it on stderr on exit, and `rusty-mux-ctl` prints it on stderr.
- **Known limitations:** no version floor yet: the server refuses no client by version. Every crate is still 0.1.0, so the feature names, not the versions, show what is missing.

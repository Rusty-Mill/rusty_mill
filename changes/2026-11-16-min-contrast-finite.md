---
category: Fixed
changelog: minimum_contrast = nan (or inf) in config.toml is now refused with a warning instead of forcing all text to black or white.
---
## 2026-11-16 - minimum_contrast refuses non-finite values

- **Fixed:** `minimum_contrast = nan` passed `clamp` and made every glyph black or white. NaN and infinity now warn and leave the key unset.

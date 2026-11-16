# Theme files (C61 T1)

Card #1534. Rows: gui-design D73, D74; handoff D297, D299 (Proposed, [C61-THEMES.md](C61-THEMES.md)). This page is the schema T2 (import), T3 (contrast) and T5 (settings UI) build on.

## Format

`themes/<name>.toml` beside the config file. It is the config's `[colors]` section on its own:

```toml
[colors]
foreground = "#d8d8d8"
background = "#1d1f21"
cursor     = "#aeafad"   # also the chrome accent (D34)
color0     = "#282a2e"   # color0 .. color15
```

- sRGB `#RRGGBB` only (D74). Keys left out inherit the default theme.
- The file name stem is the theme name, used exactly as written in `theme`, `theme_dark`, `theme_light` and `[profile.x] theme`. Names with `/`, `\`, `:` or a leading `.` never name a file.
- A user file shadows a built-in preset of the same name.
- Any bad line rejects the whole file, and so does a file with no colours. The error names the file and line.
- No `accent` key: the accent is the theme cursor, with the D35 floor and verdigris fallback (D34, D35). `chrome.rs` tests the floor over every preset and sample user files.

## Reload

The config watcher also watches `themes/` (new, edited and removed files). A theme that no longer resolves (bad file, unknown name) writes one warning line to stderr, like every config warning, and the window keeps the theme it has. The visible notice (oxide plus words, D17) is drawn in T5a; no frame exists yet.

Code: `crates/libs/ui/rusty_term/src/config/theme_file.rs`; `Config::theme_failed` carries the keep-old signal to the GUI and TUI reload paths.

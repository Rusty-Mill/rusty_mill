# rusty_term chrome style guide

Status: adopted by the owner 2026-11-03 (fixed chrome scale). Applies to everything the
windowed front-end (`--gui`) draws that is not program output: the tab strip and caption
buttons, the status ribbon, gutter command marks, the command dock, the search bar, the
settings page and menus, and later the mux client's indicators, inbox and palette (C21, C08).

Informed by Orca's MIT-licensed style guide (`stablyai/orca` `docs/STYLEGUIDE.md`, read
2026-11-03); the rules here are our own wording. Code paths are under
`crates/libs/ui/rusty_term/src/gui/`.

This guide describes the chrome as built. The GUI design pack (`docs/projects/term/gui-design/`)
is the target; "Target:" lines mark where the two differ until the pack's implementation
handoff lands (pack D31).

## Identity

- **Quiet.** Neutral surfaces and text carry the chrome; the terminal is the subject.
- **Colour means state, never decoration:** focus and active (the accent), success, failure,
  running, attention, search matches, and "changed from default".
- **One look per role.** Things that do the same job look and act the same; a difference is
  fixed on both or written down.

## Colour

Chrome colours come from `Chrome::from_theme` (`chrome.rs`), never from literals in drawing
code (pack D34 to D39):

| Role | Source |
| --- | --- |
| Surfaces: strip `frame`, active tab and pages `base`, hover and search `raised`, ribbon and dock `sunk`, hairlines `line` | Fixed pack tokens, D5 dark or D25 light, picked by the theme background (relative luminance under 0.179 is dark, D39) |
| Text `text`, `muted`; non-text icons only `dim` (D17) | Fixed pack tokens |
| Accent (active tab underline, selection bar, changed-value dot, running mark, branch) | `theme.cursor`, or pack verdigris when it is under 3:1 on any chrome surface or no pack text holds 4.5:1 on it (D35) |
| Success / failure | Pack verdigris / oxide, never the theme palette (D36) |
| Search match / active match | `SearchColors::from_theme` (`render.rs`): whichever of the theme's yellow and the fixed amber stands out more from the background, and the same between a red-yellow orange from the theme and the fixed orange; the active match always differs from the rest; glyphs keep 4.5:1 |

- A surface and its text are always used as a pair, and chrome text keeps 4.5:1 against its
  surface. Failure text never sits on `raised` (D17).
- Program output is not chrome: the grid and its padding keep the theme's colours.
- Gutter command marks sit on the theme background (a pane at the left edge) or on the divider
  (a pane right of a split), so `Marks::on` resolves them against that backing: the pack status
  token (either mode) that stands out more, the theme cursor for running when it is visible,
  nudged to 3:1 only where no pack token can reach it (D41, D42).
- A colour outside these tokens is allowed only with a stated reason and a test, as the search
  fallback and the native-red caption close hover have.

## Size and layout

- **Fixed chrome scale (owner, 2026-11-03).** Font zoom (Ctrl+= / Ctrl+-) and the configured
  terminal `font-size` change only the grid. The chrome is sized by display DPI and,
  when added, a `[window] ui_scale`. The bars draw in their own UI font (`ui_font`, #1388)
  at a fixed 18 logical px (`UI_FONT_PX`) times the display scale; neither zoom nor the
  configured `font-size` changes it.
- **Face.** The chrome uses the terminal's monospace face.
  Target: interface text in Instrument Sans, terminal text in JetBrains Mono (pack D6).
- **Spacing.** Chrome sits on a 4 px grid inside the padding band (`[window] padding`,
  default 8 px); gutter marks stay in that band.
- **Elevation, three levels:** flat (surface plus a hairline) for the strip, ribbon and dock;
  raised for menus, the search bar and sidebars; floating for popovers over the grid.

## Components

| Component | Rule |
| --- | --- |
| Tab | Muted label when idle, raised on hover; the active tab is bold with an accent underline. An unseen alert is a dot, not a coloured tab. The × shows on the active and hovered tab |
| List row | Transparent idle, raised on hover; keyboard selection is an accent bar plus a tint; the current item an accent dot. No other "selected" colour |
| Status pill | ✓ success, ✗ failure, accent while running: a glyph always goes with the colour |
| Shortcut label | Drawn from the live keymap, never typed text; never on Cancel, Close or Dismiss |
| Notice | A transient result (copied, reloaded) fades by itself; an error the user must act on stays inline until dismissed |
| Prompt | Blocking decisions only. Anything asking for trust (multi-line paste, OSC 52 read, opening a link in another app, C63) uses one trust surface themes cannot restyle |

Target: states follow the pack's state matrix (D26). The focus ring is 2 px with a 2 px offset
in the theme accent, at 3:1 or better (D27, D34). First-run setup reuses the settings overlay
with numbered steps (D32, D33).

## UX rules

- **Copy never overclaims.** Result words ("copied", "saved", "reloaded") only once the result
  is real. No "please", "simply" or "just". A setting's description is one line of at most
  80 characters; an error names what failed and one action.
- **Feedback matches duration:** nothing under 100 ms; disabled up to 1 s; a spinner glyph or
  label swap up to 3 s; stage labels beyond. Disable at once, show loading after about 200 ms,
  and reserve the width a label swap needs.
- **Remote is normal.** Once the GUI attaches to rusty_mux, chrome flows are checked under
  50 to 200 ms of added latency.
- **Keyboard first.** Every chrome action has a keymap action. Enter runs the primary action,
  Esc backs out, default focus lands on the likely field, long lists are searchable.
- **Back-out is quiet:** Close, Cancel and Dismiss get no colour, chip or animation.
- **Platform labels match the binding** (Ctrl+Shift on Linux and Windows; ⌘ and ⇧ on macOS).
- **Motion only for continuity**, and none under the OS reduced-motion setting.
  Target: only a spinner and a blinking cursor animate; under reduced motion both go static
  (pack D16, D27).
- **Missing data offers a way to get it** (no shell found leads to the shell picker).

## Enforcement and review

1. Tests for theme-derived colours run over every built-in preset (`config::PRESETS`) and a
   sweep of custom palettes, as `search_colours_stay_legible_on_every_preset` and
   `chrome_is_legible_on_every_preset` do.
2. Chrome PRs attach xvfb screenshots in one dark and one light theme, CPU and GPU.
3. `chrome.rs` tests every chrome surface and text pair for contrast. Planned: a ratchet test
   counting colour literals in `src/gui` outside derivation code.
4. Review a chrome PR for: the top three fixes; clutter, alignment, copy and focus; whether the
   main flow takes one or two actions with Enter and Esc right; empty and error states; light
   and dark; Windows and Linux.

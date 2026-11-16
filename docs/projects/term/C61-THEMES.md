# C61 split: client themes, materials, fonts, shaders

Card #690, mux capability C61, [REX-APP](../mux/REX-APP.md) I5-I8. Nano's go: 2026-11-16, "Go on arc 2, with parallel threads where possible". Every child is a Mill Board card with parent #690. Design review: `rusty-term-design-expert`, 2026-11-16. Rows: gui-design D73-D76, Q74-Q76; handoff D297-D302.

Already in `rusty_term`: `theme` presets and `theme_auto/dark/light`, `minimum_contrast`, `opacity` (GPU only), `font`, bold/italic faces, `font_fallback` (one file), `ligatures`. `wgpu` 22 is already a dependency of `gui-gpu`.

## Children

| Card | Child | Frames | Paths | Needs |
|---|---|---|---|---|
| T1 | Theme model, TOML theme files, live reload, accent hook (D34, D35 sweep over every theme) | no | CPU + GPU | nothing; its schema is the contract for T2, T3, T5 |
| T2 | Import Ghostty (key=value) and iTerm2 (`.itermcolors` plist) into T1 files; pure parser, hand-written, no plist crate | no | n/a | T1 schema |
| T3 | Contrast audit: `minimum_contrast` on CPU and GPU, chrome exempt (D76). Settings row moves to T5 | no | CPU + GPU | nothing |
| T4a | Fonts: fallback chain (list), weight, line height, letter spacing; cell-metric check on both paths | no | CPU + GPU | nothing |
| T4b | Font settings rows on the Terminal page (D30) | yes | CPU + GPU | T4a, frames |
| T5a | Frames: theme picker with preview, light/dark pairing, contrast row | yes (Paper) | n/a | Nano's machine |
| T5b | Theme settings UI built from T5a | yes | CPU + GPU | T1, T3, T5a |
| T6 | Materials: opacity plus blur, visible fallback. Scope per Q75 | yes | GPU only, OS-dependent | T5b, frames |
| T7 | Custom post-process shaders, WGSL, safe failure to unshaded | minimal | GPU only | Nano's answer to Q76 |

## Build order

T1, with T3 and T4a beside it, then T2 and T4b, then T5a and T5b, then T6, then T7 if shipped. Safe to run in parallel now: T1, T3, T4a; T2 once the T1 schema merges. Frames for T5 and T6 are one Paper session.

## Gates

- No new dependency or lockfile change in T1-T6. T7 with GLSL (naga `glsl-in`) needs Nano's approval (Q76).
- Each child ships tests and dark + light, CPU + GPU screenshots (GPU-only children: GPU only, with the CPU fallback shown).
- C61 is done in CAPABILITIES when T1-T6 land and T7 has landed or Nano has declined it (Q76). T7 is the only optional child.

## Questions for Nano (interim choice in brackets)

- Q74: in-app colour customizer, or pick, preview and pair only? [pick only; edit the file by hand]
- Q75: materials as opacity plus blur only, or named pane styles? [opacity plus blur]
- Q76: ship shaders at all, WGSL only or GLSL? [not started; WGSL, no new dependency]

---
category: Changed
changelog: rusty_term --gui chrome shows a pressed state on tabs and buttons, and disabled settings rows draw faded with no hover.
---
## 2026-11-05 - rusty_term chrome pressed and disabled states

- **Changed:** while the mouse is down on a tab, a tab's ×, `+`, `▾` or a caption button, the
  control draws one surface step up (`line`) with full text, held through a tab drag. A disabled
  settings row draws its text at 45% over its surface and no longer lights up on hover (design
  pack D26, D43). Second slice of the GUI redesign.
- **Known limitations:** no keyboard focus ring yet; the tab strip and caption buttons take no
  keyboard focus, and list rows keep the accent bar as their focus mark (D43). Reduced motion and
  the Instrument Sans interface font are later slices.

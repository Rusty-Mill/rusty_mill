---
category: Added
changelog: rusty_term --gui honours reduced motion, from the OS setting or `[window] reduce_motion`, with a solid cursor and no cursor trail.
---
## 2026-11-05 - rusty_term reduced motion

- **Added:** `[window] reduce_motion = "system" | "on" | "off"` (default `system`) and a
  "Reduce motion" row on the settings page. When reduced, the cursor draws solid even if
  `cursor_blink` or a program asks it to blink, and the cursor trail is not drawn. Kitty image
  animations keep playing, since they are program output (design pack D27, D50 to D52). Third
  slice of the GUI redesign.
- **System detection:** Windows reads `SPI_GETCLIENTAREAANIMATION`; GNOME desktops read
  `gsettings get org.gnome.desktop.interface enable-animations`; macOS reads
  `defaults read com.apple.universalaccess reduceMotion`. A missing tool, an error or a reader
  slower than 300 ms counts as no signal (motion allowed). No new dependencies.
- **Known limitations:** the OS setting is read once at startup; changing it while the terminal
  runs takes effect on the next launch (D52). Other desktops can set `reduce_motion = "on"`.

# Instrument Sans

The GUI's interface font (gui-design D6, D59), embedded by `src/gui/text.rs`.

- Source: `ofl/instrumentsans/InstrumentSans[wdth,wght].ttf` in
  [google/fonts](https://github.com/google/fonts) (upstream commit
  `7fa22308a3d0c94ee2b3cd537a1196b65db34a3e` of
  [Instrument/instrument-sans](https://github.com/Instrument/instrument-sans)).
- Licence: SIL Open Font License 1.1, in `OFL.txt`. It names no Reserved Font
  Name, so instances keep the family name.
- The two files are static instances at width 100, weights 400 and 500
  (gui-design D46): `rusty_font` reads static outlines only.

Made with fontTools 4.68:

```
fonttools varLib.instancer "InstrumentSans[wdth,wght].ttf" wght=400 wdth=100 --update-name-table -o InstrumentSans-Regular.ttf
fonttools varLib.instancer "InstrumentSans[wdth,wght].ttf" wght=500 wdth=100 --update-name-table -o InstrumentSans-Medium.ttf
```

---
category: Fixed
changelog: rusty_conhost: with wrap off, legacy text past the right margin goes back to where its run started, as conhost does (#1336).
---
## 2026-11-01 - Console host text past the margin with wrap off

- **Fixed:** with `ENABLE_WRAP_AT_EOL_OUTPUT` and VT processing off, our console host left text that overflowed the row in the last column. conhost (`_writeCharsLegacyUnprocessed`) sends the cursor back to the column the run of text started at and overwrites the row from there; a run is the whole write without `ENABLE_PROCESSED_OUTPUT`, else the text between controls. Our host now does the same and tells the terminal with a CHA, so the terminal shows what the buffer holds. A double-width glyph that would cross the margin pads the last column with a space and moves to the run's start, or is dropped when the run started in the last column, also as conhost does. The parity suite's `print-x 1 205` probe now agrees with ConPTY (#1336).
- **Known limitations:** a tab that reaches the margin with wrap off still stops in the last column; conhost writes it as spaces and goes back to the tab's column.

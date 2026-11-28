---
category: Fixed
changelog: Mill Board web UI - the column strip no longer overlaps the column headers.
---
## 2026-11-27 - Mill Board: column strip overlapped the column headers

- **Fixed:** the page was a fixed four-row grid, but the date filter (#1098) made five children, so the colour strip landed in the stretch row and the board in an implicit one. The strip's labels were drawn over the headers and the first card was clipped. The page is now a flex column: bars take their height, the board takes the rest, and a hidden bar can no longer shift the rows.
- **Known limitations:** none.

// Small monoline SVG icons, 16x16, `stroke="currentColor"` so a button's
// own `color` tints them -- no icon font or extra dependency for a
// handful of glyphs this small.

function svg(inner) {
  return `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
}

export const ICONS = {
  plus: svg('<path d="M8 3v10M3 8h10"/>'),
  close: svg('<path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/>'),
  rename: svg(
    '<path d="M3 13l.8-3 6.8-6.8a1.2 1.2 0 0 1 1.7 0l.5.5a1.2 1.2 0 0 1 0 1.7L6 12.2 3 13z"/><path d="M9.3 4l2.7 2.7"/>',
  ),
  fork: svg(
    '<circle cx="4" cy="3.5" r="1.4"/><circle cx="12" cy="3.5" r="1.4"/><circle cx="8" cy="12.5" r="1.4"/><path d="M4 4.9v1.6a2 2 0 0 0 2 2h0"/><path d="M12 4.9v1.6a2 2 0 0 1-2 2h0"/><path d="M8 8.5v2.6"/>',
  ),
  switchAgent: svg(
    '<path d="M2.5 6h8.5M9 3.2L11.5 6 9 8.8"/><path d="M13.5 10h-8.5M7 12.8L4.5 10 7 7.2"/>',
  ),
  diff: svg('<rect x="2" y="2.5" width="5.5" height="11" rx="1.3"/><rect x="8.5" y="2.5" width="5.5" height="11" rx="1.3"/>'),
  branch: svg(
    '<circle cx="4" cy="3.2" r="1.3"/><circle cx="4" cy="12.8" r="1.3"/><circle cx="12" cy="6.8" r="1.3"/><path d="M4 4.5v6.8"/><path d="M4 7.5a3.3 3.3 0 0 0 3.3 3.3H10"/>',
  ),
  search: svg('<circle cx="6.7" cy="6.7" r="4"/><path d="M9.7 9.7L13.3 13.3"/>'),
  chevronDown: svg('<path d="M4 6l4 4 4-4"/>'),
  grid: svg(
    '<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>',
  ),
  expand: svg('<rect x="2.5" y="2.5" width="11" height="11" rx="1.5"/>'),
  clock: svg('<circle cx="8" cy="8" r="5.5"/><path d="M8 5.3v3l2 1.2"/>'),
  stop: svg('<rect x="4.2" y="4.2" width="7.6" height="7.6" rx="1.3" fill="currentColor" stroke="none"/>'),
};

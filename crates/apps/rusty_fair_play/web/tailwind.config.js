/** Colours come from CSS variables (src/styles/tokens.css) so a dark theme is a variable swap. */
const v = (name) => `rgb(var(--${name}) / <alpha-value>)`

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  darkMode: ['class', '[data-theme="dark"]'],
  theme: {
    extend: {
      colors: {
        text: v('text'),
        grey: v('grey'),
        line: v('line'),
        surface: v('surface'),
        paper: v('paper'),
        rail: v('rail'),
        side: v('side'),
        hover: v('hover'),
        selected: v('selected'),
        primary: v('primary'),
        green: v('green'),
        danger: v('danger'),
        amber: v('amber'),
        violet: v('violet'),
        'suit-home': v('suit-home'),
        'suit-out': v('suit-out'),
        'suit-caregiving': v('suit-caregiving'),
        'suit-magic': v('suit-magic'),
        'suit-wild': v('suit-wild'),
        'suit-unicorn': v('suit-unicorn'),
      },
      fontSize: { xs: ['11px', '16px'], s: ['12px', '18px'], base: ['14px', '21px'], title: ['18px', '26px'], h1: ['20px', '28px'] },
      borderRadius: { row: '8px', menu: '12px', dialog: '16px', card: '10px' },
      boxShadow: { pop: '0 4px 16px rgba(0,0,0,.12)', card: '0 1px 2px rgba(0,0,0,.06)' },
      transitionDuration: { DEFAULT: '150ms' },
    },
  },
  plugins: [],
}

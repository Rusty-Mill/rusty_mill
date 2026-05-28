// Bundled themes. Each maps the `--color-rk-*` custom properties Tailwind v4
// generates from `@theme`; switching overrides them on the document root at
// runtime and persists the choice. More themes can be added to this table.

export interface Theme {
  name: string;
  vars: Record<string, string>;
}

export const THEMES: Theme[] = [
  {
    name: "Rust Dark",
    vars: {
      "--color-rk-bg": "#0f1115",
      "--color-rk-panel": "#161922",
      "--color-rk-panel-2": "#1d212c",
      "--color-rk-border": "#2a2f3a",
      "--color-rk-text": "#d7dce5",
      "--color-rk-muted": "#8b93a3",
      "--color-rk-gold": "#d4a017",
    },
  },
  {
    name: "Midnight",
    vars: {
      "--color-rk-bg": "#0a0e1a",
      "--color-rk-panel": "#0f1626",
      "--color-rk-panel-2": "#152033",
      "--color-rk-border": "#22314d",
      "--color-rk-text": "#cdd6f4",
      "--color-rk-muted": "#7f8bb0",
      "--color-rk-gold": "#89b4fa",
    },
  },
  {
    name: "Solarized Night",
    vars: {
      "--color-rk-bg": "#002b36",
      "--color-rk-panel": "#073642",
      "--color-rk-panel-2": "#0a4250",
      "--color-rk-border": "#0f5666",
      "--color-rk-text": "#eee8d5",
      "--color-rk-muted": "#93a1a1",
      "--color-rk-gold": "#b58900",
    },
  },
  {
    name: "Paper Light",
    vars: {
      "--color-rk-bg": "#f6f5f1",
      "--color-rk-panel": "#ffffff",
      "--color-rk-panel-2": "#eceae3",
      "--color-rk-border": "#d6d3c8",
      "--color-rk-text": "#23252b",
      "--color-rk-muted": "#6b6f76",
      "--color-rk-gold": "#a86b00",
    },
  },
];

const KEY = "rk.theme";

export function applyTheme(name: string) {
  const theme = THEMES.find((t) => t.name === name) ?? THEMES[0];
  for (const [k, v] of Object.entries(theme.vars)) {
    document.documentElement.style.setProperty(k, v);
  }
  try {
    localStorage.setItem(KEY, theme.name);
  } catch (_) {
    /* storage may be unavailable */
  }
}

export function loadTheme(): string {
  try {
    return localStorage.getItem(KEY) ?? THEMES[0].name;
  } catch (_) {
    return THEMES[0].name;
  }
}

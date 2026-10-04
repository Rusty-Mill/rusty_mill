/** System, light or dark: the choice is remembered; "system" follows prefers-color-scheme live. */
import { create } from 'zustand'

export type Theme = 'system' | 'light' | 'dark'
const KEY = 'fair-play:theme'

const read = (): Theme => {
  try {
    const t = localStorage.getItem(KEY)
    return t === 'light' || t === 'dark' ? t : 'system'
  } catch {
    return 'system'
  }
}

export const resolveTheme = (theme: Theme): 'light' | 'dark' =>
  theme === 'system' ? (typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : theme

export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = resolveTheme(theme)
}

interface ThemeState {
  theme: Theme
  set(theme: Theme): void
}

export const useTheme = create<ThemeState>()((set) => ({
  theme: read(),
  set(theme) {
    set({ theme })
    try {
      localStorage.setItem(KEY, theme)
    } catch {
      /* storage unavailable: lasts until reload */
    }
    applyTheme(theme)
  },
}))

/** Apply the saved theme now and keep "system" in step with the OS. Returns a stop function. */
export function startTheme(): () => void {
  applyTheme(useTheme.getState().theme)
  if (typeof matchMedia !== 'function') return () => undefined
  const m = matchMedia('(prefers-color-scheme: dark)')
  const on = (): void => applyTheme(useTheme.getState().theme)
  m.addEventListener('change', on)
  return () => m.removeEventListener('change', on)
}

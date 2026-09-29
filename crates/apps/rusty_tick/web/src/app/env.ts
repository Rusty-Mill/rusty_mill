/**
 * Which backend the UI talks to, and the token for it.
 *
 * `server`: the real `rusty_tick` API, same origin, bearer token.
 * `demo`: the in-browser MemoryAdapter with sample data, so the UI runs alone.
 *
 * The token lives in sessionStorage by default (gone when the tab closes) and
 * in localStorage only if the user ticks "remember on this device".
 */
export type Mode = 'server' | 'demo'

const MODE_KEY = 'tick-local:mode'
const TOKEN_KEY = 'tick-local:token'

const safe = <T>(fn: () => T, fallback: T): T => {
  try {
    return fn()
  } catch {
    return fallback // storage blocked (private mode, sandbox): behave as if empty
  }
}

export function getMode(): Mode | null {
  const url = new URLSearchParams(location.search).get('adapter')
  if (url === 'memory' || url === 'demo') return 'demo'
  const stored = safe(() => localStorage.getItem(MODE_KEY), null)
  return stored === 'server' || stored === 'demo' ? stored : null
}

export function setMode(mode: Mode): void {
  safe(() => localStorage.setItem(MODE_KEY, mode), undefined)
}

export function getToken(): string | null {
  return safe(() => sessionStorage.getItem(TOKEN_KEY) ?? localStorage.getItem(TOKEN_KEY), null)
}

export function setToken(token: string, remember: boolean): void {
  safe(() => {
    sessionStorage.setItem(TOKEN_KEY, token)
    if (remember) localStorage.setItem(TOKEN_KEY, token)
    else localStorage.removeItem(TOKEN_KEY)
  }, undefined)
}

export function clearToken(): void {
  safe(() => {
    sessionStorage.removeItem(TOKEN_KEY)
    localStorage.removeItem(TOKEN_KEY)
  }, undefined)
}

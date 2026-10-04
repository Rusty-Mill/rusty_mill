/**
 * Which backend the UI talks to, and the token for it.
 *
 * `server`: the real `rusty_fair_play` API, same origin, optional bearer token.
 * `demo`: the in-browser MemoryAdapter (`?adapter=memory`), so the UI runs alone.
 *
 * The token lives in sessionStorage by default (gone when the tab closes) and
 * in localStorage only if the user ticks "remember on this device".
 */
export type Mode = 'server' | 'demo'

const TOKEN_KEY = 'fair-play:token'

const safe = <T>(fn: () => T, fallback: T): T => {
  try {
    return fn()
  } catch {
    return fallback // storage blocked (private mode, sandbox): behave as if empty
  }
}

export function getMode(): Mode {
  const url = new URLSearchParams(location.search).get('adapter')
  return url === 'memory' || url === 'demo' ? 'demo' : 'server'
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

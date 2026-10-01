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
const WHO_KEY = 'tick-local:who'
// Everything else under `tick-local:` and `server:` is one user's cached data.
const KEEP = new Set([MODE_KEY, TOKEN_KEY, WHO_KEY, 'tick-local:ui:v1', 'tick-local:memory:v1'])

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

/** Who a token is for: the user key of a `<user>.<secret>` token (ADR-0002), empty for the single-user token. */
export const identity = (token: string | null): string => (token?.includes('.') ? token.slice(0, token.indexOf('.')) : '')

/** Drop the cached tasks, queue and preferences of whoever used this browser before. */
export function forgetUserData(): void {
  safe(() => {
    for (const k of Object.keys(localStorage)) if ((k.startsWith('tick-local:') || k.startsWith('server:')) && !KEEP.has(k)) localStorage.removeItem(k)
  }, undefined)
}

/**
 * Cached data belongs to one user. If the token now in use names a different
 * user than the last one, forget the last user's data so it is neither shown
 * to the new user nor replayed into their store. True when it did.
 */
export function adoptIdentity(token: string | null): boolean {
  const now = identity(token)
  const before = safe(() => localStorage.getItem(WHO_KEY) ?? '', '')
  if (now === before) return false
  forgetUserData()
  safe(() => localStorage.setItem(WHO_KEY, now), undefined)
  return true
}

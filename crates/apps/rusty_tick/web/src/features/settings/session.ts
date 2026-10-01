/**
 * Leaving the session: sign out, reset the demo, leave the demo. Each ends with
 * a page reload so the app boots into whichever gate now applies.
 */
import { STORAGE_KEY } from '@/api/memory'
import { clearToken, forgetUserData } from '@/app/env'

const MODE_KEY = 'tick-local:mode' // env.ts keeps this private and offers no way to unset it

/** Indirection so tests can observe the reload (jsdom cannot perform one). */
export const page = {
  reload: (): void => location.reload(),
  /** Load `url` in place of this page (a plain reload would keep `?adapter=memory`, which forces demo mode). */
  replace: (url: string): void => location.replace(url),
}

export function signOut(): void {
  clearToken()
  forgetUserData() // a shared browser must not keep the last user's tasks
  page.reload()
}

/** Forget the demo's tasks so the next start reseeds the sample data. Preferences are kept. */
export function resetDemoData(): void {
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    /* storage blocked: nothing was saved to remove */
  }
  page.reload()
}

/** Back to the welcome screen, where the mode is chosen again. */
export function leaveDemo(): void {
  try {
    localStorage.removeItem(MODE_KEY) // no stored mode is what shows the welcome screen
  } catch {
    /* storage blocked */
  }
  page.replace(location.pathname)
}

/**
 * Keyboard shortcuts: which key means what. Kept apart from the hook that
 * listens so the mapping, and the rule for when typing suppresses it, can be
 * tested.
 */
import type { Priority } from '@/api/types'

export type ShortcutAction =
  | { type: 'newTask' }
  | { type: 'next' }
  | { type: 'previous' }
  | { type: 'toggle' }
  | { type: 'delete' }
  | { type: 'search' }
  | { type: 'close' }
  | { type: 'priority'; value: Priority }

export interface KeyLike {
  key: string
  ctrlKey: boolean
  metaKey: boolean
  altKey: boolean
  shiftKey: boolean
  target: { tagName?: string; isContentEditable?: boolean; getAttribute?: (n: string) => string | null } | null
}

/** True when the key press is going into a text field (or a control that uses the same keys). */
export function isTyping(target: KeyLike['target']): boolean {
  if (!target) return false
  const tag = target.tagName?.toUpperCase()
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true
  if (target.isContentEditable) return true
  const role = target.getAttribute?.('role')
  return role === 'textbox' || role === 'combobox'
}

const PRIORITY_KEYS: Record<string, Priority> = { '1': 5, '2': 3, '3': 1, '4': 0 }

export function shortcutFor(e: KeyLike): ShortcutAction | null {
  const mod = e.ctrlKey || e.metaKey
  // Search works anywhere, even while typing.
  if (mod && !e.altKey && e.key.toLowerCase() === 'k') return { type: 'search' }
  if (e.key === 'Escape') return { type: 'close' }
  if (mod || e.altKey || isTyping(e.target)) return null
  switch (e.key) {
    case 'n':
    case 'N':
      return { type: 'newTask' }
    case 'j':
    case 'J':
      return { type: 'next' }
    case 'k':
    case 'K':
      return { type: 'previous' }
    case ' ':
      return { type: 'toggle' }
    case 'Delete':
    case 'Backspace':
      return { type: 'delete' }
  }
  const p = PRIORITY_KEYS[e.key]
  return p === undefined ? null : { type: 'priority', value: p }
}

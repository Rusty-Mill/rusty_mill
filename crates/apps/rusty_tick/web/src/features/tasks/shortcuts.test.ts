import { describe, expect, it } from 'vitest'
import { isTyping, shortcutFor, type KeyLike } from './shortcuts'

const press = (key: string, over: Partial<KeyLike> = {}): KeyLike => ({ key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, target: { tagName: 'BODY' }, ...over })

describe('shortcutFor', () => {
  it('maps the documented keys', () => {
    expect(shortcutFor(press('n'))).toEqual({ type: 'newTask' })
    expect(shortcutFor(press('j'))).toEqual({ type: 'next' })
    expect(shortcutFor(press('k'))).toEqual({ type: 'previous' })
    expect(shortcutFor(press(' '))).toEqual({ type: 'toggle' })
    expect(shortcutFor(press('Delete'))).toEqual({ type: 'delete' })
    expect(shortcutFor(press('Escape'))).toEqual({ type: 'close' })
  })

  it('1-4 set priority high, medium, low, none', () => {
    expect(shortcutFor(press('1'))).toEqual({ type: 'priority', value: 5 })
    expect(shortcutFor(press('2'))).toEqual({ type: 'priority', value: 3 })
    expect(shortcutFor(press('3'))).toEqual({ type: 'priority', value: 1 })
    expect(shortcutFor(press('4'))).toEqual({ type: 'priority', value: 0 })
    expect(shortcutFor(press('5'))).toBeNull()
  })

  it('Ctrl or Cmd + K is search, even from a text field', () => {
    expect(shortcutFor(press('k', { ctrlKey: true }))).toEqual({ type: 'search' })
    expect(shortcutFor(press('K', { metaKey: true, target: { tagName: 'INPUT' } }))).toEqual({ type: 'search' })
  })

  it('does not steal keys while typing', () => {
    for (const tagName of ['INPUT', 'TEXTAREA', 'SELECT']) {
      expect(shortcutFor(press('n', { target: { tagName } }))).toBeNull()
      expect(shortcutFor(press(' ', { target: { tagName } }))).toBeNull()
      expect(shortcutFor(press('Delete', { target: { tagName } }))).toBeNull()
    }
    expect(shortcutFor(press('j', { target: { tagName: 'DIV', isContentEditable: true } }))).toBeNull()
    expect(shortcutFor(press('1', { target: { tagName: 'DIV', getAttribute: (n) => (n === 'role' ? 'textbox' : null) } }))).toBeNull()
  })

  it('Escape still closes things while typing', () => {
    expect(shortcutFor(press('Escape', { target: { tagName: 'INPUT' } }))).toEqual({ type: 'close' })
  })

  it('leaves modified keys to the browser', () => {
    expect(shortcutFor(press('n', { ctrlKey: true }))).toBeNull()
    expect(shortcutFor(press('j', { metaKey: true }))).toBeNull()
    expect(shortcutFor(press('1', { altKey: true }))).toBeNull()
  })

  it('ignores everything else', () => {
    expect(shortcutFor(press('x'))).toBeNull()
    expect(shortcutFor(press('Enter'))).toBeNull()
  })
})

describe('isTyping', () => {
  it('is false for no target and for ordinary elements', () => {
    expect(isTyping(null)).toBe(false)
    expect(isTyping({ tagName: 'BUTTON' })).toBe(false)
  })
})

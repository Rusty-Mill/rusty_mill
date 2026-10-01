import { describe, expect, it } from 'vitest'
import { normalizeChecklists, normalizeUrl } from './editorCommands'

describe('normalizeUrl', () => {
  it('adds https to a bare domain and mailto to an address', () => {
    expect(normalizeUrl('example.com/a')).toBe('https://example.com/a')
    expect(normalizeUrl('  http://x.dev ')).toBe('http://x.dev')
    expect(normalizeUrl('me@example.com')).toBe('mailto:me@example.com')
  })
  it('rejects anything else', () => {
    expect(normalizeUrl('')).toBeNull()
    expect(normalizeUrl('two words')).toBeNull()
    expect(normalizeUrl('javascript:alert(1)')).toBeNull()
    expect(normalizeUrl('data:text/html,hi')).toBeNull()
  })
})

describe('normalizeChecklists', () => {
  it('gives every checklist item a state, leaving ticked ones alone', () => {
    const root = document.createElement('div')
    root.innerHTML = '<ul data-checklist><li>a</li><li data-checked="true">b</li><li data-checked="maybe">c</li></ul><ul><li>plain</li></ul>'
    normalizeChecklists(root)
    expect([...root.querySelectorAll('li')].map((l) => l.getAttribute('data-checked'))).toEqual(['false', 'true', 'false', null])
  })
})

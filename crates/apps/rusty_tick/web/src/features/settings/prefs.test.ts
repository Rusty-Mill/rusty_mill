import { beforeEach, describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import { DEFAULT_PREFS, PREFS_ID, resolveTheme, sanitize, usePrefs } from './prefs'

describe('sanitize', () => {
  it('keeps good values and defaults the rest', () => {
    expect(sanitize({ weekStart: 0, hour12: true, theme: 'dark', defaultReminder: 'TRIGGER:-PT1H', notifications: true })).toEqual({
      weekStart: 0, hour12: true, theme: 'dark', defaultReminder: 'TRIGGER:-PT1H', notifications: true,
    })
    expect(sanitize({ weekStart: 3, hour12: 'yes', theme: 'neon', defaultReminder: 5, notifications: 'yes' })).toEqual(DEFAULT_PREFS)
    expect(sanitize(null)).toEqual(DEFAULT_PREFS)
    expect(sanitize('junk')).toEqual(DEFAULT_PREFS)
  })
})

describe('resolveTheme', () => {
  it('passes light and dark through', () => {
    expect(resolveTheme('dark')).toBe('dark')
    expect(resolveTheme('light')).toBe('light')
  })
})

describe('usePrefs', () => {
  beforeEach(() => usePrefs.setState({ prefs: DEFAULT_PREFS }))

  it('saves locally and applies the theme', () => {
    usePrefs.getState().update({ theme: 'dark', hour12: true })
    expect(usePrefs.getState().prefs).toMatchObject({ theme: 'dark', hour12: true })
    expect(document.documentElement.dataset.theme).toBe('dark')
    expect(JSON.parse(localStorage.getItem('tick-local:prefs:v1')!)).toMatchObject({ theme: 'dark' })
  })

  it('refuses an invalid patch instead of storing it', () => {
    usePrefs.getState().update({ weekStart: 4 as never })
    expect(usePrefs.getState().prefs.weekStart).toBe(DEFAULT_PREFS.weekStart)
  })

  it('adopts the server copy', async () => {
    const api = new MemoryAdapter()
    await api.putDoc('prefs', PREFS_ID, { weekStart: 0, theme: 'dark', hour12: true, defaultReminder: 'TRIGGER:PT0S' })
    await usePrefs.getState().load(api)
    expect(usePrefs.getState().prefs).toMatchObject({ weekStart: 0, theme: 'dark' })
  })

  it('keeps local prefs when the server has none or is unreachable', async () => {
    usePrefs.getState().update({ hour12: true })
    await usePrefs.getState().load(new MemoryAdapter())
    expect(usePrefs.getState().prefs.hour12).toBe(true)
    const broken = { listDocs: () => Promise.reject(new Error('down')) } as never
    await expect(usePrefs.getState().load(broken)).resolves.toBeUndefined()
  })
})

import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { INBOX_ID, MemoryAdapter } from '@/api/memory'
import { ServicesProvider } from '@/app/services'
import { usePrefs } from '@/features/settings/prefs'
import { createDataStore } from '@/store/data'
import { Reminders } from './Reminders'

const MIN = 60_000
const shown: string[] = []

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
  vi.setSystemTime(0)
  shown.length = 0
  vi.stubGlobal('Notification', Object.assign(function (this: unknown, title: string) { shown.push(title) }, { permission: 'granted' }))
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

async function mount(on: boolean) {
  const api = new MemoryAdapter({ now: () => 0 })
  for (const [title, at] of [['first', 5], ['second', 10]] as const) await api.createTask({ listId: INBOX_ID, title, dueMs: at * MIN, reminders: ['TRIGGER:PT0S'] })
  const store = createDataStore({ api, now: () => 0 })
  await store.getState().boot()
  usePrefs.getState().update({ notifications: on })
  render(<ServicesProvider services={{ mode: 'demo', api, store }}><Reminders /></ServicesProvider>)
}

describe('Reminders', () => {
  it('notifies as each reminder comes due, one after the other', async () => {
    await mount(true)
    await act(() => vi.advanceTimersByTimeAsync(5 * MIN))
    expect(shown).toEqual(['first'])
    await act(() => vi.advanceTimersByTimeAsync(5 * MIN))
    expect(shown).toEqual(['first', 'second'])
  })
  it('stays quiet when the user has not turned it on', async () => {
    await mount(false)
    await act(() => vi.advanceTimersByTimeAsync(20 * MIN))
    expect(shown).toEqual([])
  })
  it('stays quiet when the browser has not allowed it', async () => {
    ;(Notification as unknown as { permission: string }).permission = 'denied'
    await mount(true)
    await act(() => vi.advanceTimersByTimeAsync(20 * MIN))
    expect(shown).toEqual([])
  })
})

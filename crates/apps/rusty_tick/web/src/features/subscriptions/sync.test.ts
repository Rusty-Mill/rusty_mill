import { describe, expect, it } from 'vitest'
import { MemoryAdapter } from '@/api/memory'
import type { NewTask, Task, TaskPatch } from '@/api/types'
import { buildEvents } from '@/features/calendar/layout'
import { dayKey, startOfDay } from '@/lib/date'
import { syncSubscription } from './sync'
import { useSubscriptions } from './store'

const event = (...lines: string[]): string => ['BEGIN:VEVENT', ...lines, 'END:VEVENT'].join('\r\n')
const cal = (...events: string[]): string => ['BEGIN:VCALENDAR', ...events, 'END:VCALENDAR'].join('\r\n')
const master = event('UID:r1', 'SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'RRULE:FREQ=WEEKLY')
const movedA = event('UID:r1', 'RECURRENCE-ID:20261012T090000Z', 'SUMMARY:Standup', 'DTSTART:20261013T090000Z')
const movedB = event('UID:r1', 'RECURRENCE-ID:20261019T090000Z', 'SUMMARY:Standup', 'DTSTART:20261020T090000Z')
const day = (iso: string): string => dayKey(startOfDay(Date.parse(iso)))

/** A feed served from `text`, synced into a MemoryAdapter through actions that write to it. */
async function harness() {
  const api = new MemoryAdapter()
  const list = await api.createList({ name: 'Feed' })
  let text = ''
  const tasks = async (): Promise<Record<string, Task>> => Object.fromEntries((await api.snapshot()).tasks.map((t) => [t.id, t]))
  const actions = {
    createTask: (input: Omit<NewTask, 'id'>) => api.createTask(input),
    updateTask: async (id: string, patch: TaskPatch) => void (await api.updateTask(id, patch)),
    trashTask: async (id: string) => void (await api.trashTask(id)),
  }
  const store = useSubscriptions.getState()
  const sync = async (feed: string): Promise<void> => {
    text = feed
    const sub = useSubscriptions.getState().items.find((s) => s.url === 'https://x/f.ics')!
    await syncSubscription(sub, { api: { fetchIcs: async () => text }, actions, tasks: await tasks() })
  }
  store.add({ name: 'Feed', url: 'https://x/f.ics', listId: list.id, items: {}, syncedMs: 0 })
  /** The local days the calendar draws Standup on, through October. */
  const shown = async (): Promise<string[]> =>
    buildEvents(Object.values(await tasks()).filter((t) => t.deletedMs === null), false, new Date(2026, 10, 1).getTime())
      .map((e) => dayKey(e.startDay))
      .sort()
  return { api, sync, shown }
}

describe('syncSubscription: repeating events', () => {
  it('shows each moved instance on its new day only, and follows the feed when it moves another', async () => {
    const h = await harness()
    await h.sync(cal(master, movedA))
    expect(await h.shown()).toEqual([day('2026-10-05T09:00Z'), day('2026-10-13T09:00Z'), day('2026-10-19T09:00Z'), day('2026-10-26T09:00Z')].sort())
    await h.sync(cal(movedB, master, movedA))
    expect(await h.shown()).toEqual([day('2026-10-05T09:00Z'), day('2026-10-13T09:00Z'), day('2026-10-20T09:00Z'), day('2026-10-26T09:00Z')].sort())
    await h.sync(cal(master, movedB))
    expect(await h.shown()).toEqual([day('2026-10-05T09:00Z'), day('2026-10-12T09:00Z'), day('2026-10-20T09:00Z'), day('2026-10-26T09:00Z')].sort())
    expect((await h.api.snapshot()).tasks.filter((t) => t.deletedMs === null)).toHaveLength(2)
  })
})

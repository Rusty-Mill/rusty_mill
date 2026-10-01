import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { computeStats } from './stats'

const at = (y: number, m: number, d: number, h = 12, min = 0): number => new Date(y, m - 1, d, h, min).getTime()
const NOW = at(2026, 9, 29, 15) // Tuesday

let n = 0
const task = (o: Partial<Task> = {}): Task => {
  n += 1
  return {
    id: `t${n}`, listId: 'work', parentId: null, title: `t${n}`, notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: null, isAllDay: false, timeZone: '',
    reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: n, createdMs: 0, updatedMs: 0, completedMs: null, deletedMs: null, etag: '1', ...o,
  }
}
const done = (completedMs: number, o: Partial<Task> = {}): Task => task({ status: 'done', completedMs, ...o })
const lists = [
  { id: 'work', name: 'Work' },
  { id: 'home', name: 'Home' },
] as never[]

describe('computeStats', () => {
  it('is all zeros for no tasks', () => {
    expect(computeStats([], lists, NOW, 1)).toEqual({ completedToday: 0, completedThisWeek: 0, completedAllTime: 0, open: 0, overdue: 0, streak: 0, topLists: [] })
  })

  it('counts completions today, this week and ever', () => {
    const tasks = [done(at(2026, 9, 29, 0, 0)), done(at(2026, 9, 28, 23, 59)), done(at(2026, 9, 27, 12)), done(at(2026, 8, 1))]
    const s = computeStats(tasks, lists, NOW, 1)
    expect(s.completedToday).toBe(1) // midnight counts, 23:59 the day before does not
    expect(s.completedThisWeek).toBe(2) // Mon 28 and Tue 29; Sunday belongs to last week
    expect(s.completedAllTime).toBe(4)
    expect(computeStats(tasks, lists, NOW, 0).completedThisWeek).toBe(3) // weeks starting on Sunday include the 27th
  })

  it('ignores trashed tasks everywhere', () => {
    const s = computeStats([done(NOW - 1000, { deletedMs: 1 }), task({ deletedMs: 1, dueMs: at(2026, 9, 1) })], lists, NOW, 1)
    expect(s).toMatchObject({ completedToday: 0, completedAllTime: 0, open: 0, overdue: 0 })
  })

  it('counts open and overdue: all-day tasks are overdue from the next day, timed ones from their time', () => {
    const tasks = [
      task({ dueMs: at(2026, 9, 28, 0), isAllDay: true }), // yesterday, all day: overdue
      task({ dueMs: at(2026, 9, 29, 0), isAllDay: true }), // today, all day: not yet
      task({ dueMs: at(2026, 9, 29, 9) }), // earlier today: overdue
      task({ dueMs: at(2026, 9, 29, 18) }), // later today: not yet
      task({}), // no date
      done(NOW, { dueMs: at(2026, 9, 1) }), // finished tasks are never overdue
    ]
    const s = computeStats(tasks, lists, NOW, 1)
    expect(s.open).toBe(5)
    expect(s.overdue).toBe(2)
  })

  describe('streak', () => {
    it('counts consecutive days ending today', () => {
      const tasks = [done(at(2026, 9, 29, 8)), done(at(2026, 9, 29, 20)), done(at(2026, 9, 28)), done(at(2026, 9, 27)), done(at(2026, 9, 25))]
      expect(computeStats(tasks, lists, NOW, 1).streak).toBe(3)
    })
    it('is not broken by a day that is not over yet', () => {
      expect(computeStats([done(at(2026, 9, 28)), done(at(2026, 9, 27))], lists, NOW, 1).streak).toBe(2)
    })
    it('is zero after a missed day', () => {
      expect(computeStats([done(at(2026, 9, 27))], lists, NOW, 1).streak).toBe(0)
    })
    it('spans a month boundary and the end of daylight saving time', () => {
      const now = at(2026, 11, 2, 10)
      const tasks = [done(at(2026, 11, 2, 9)), done(at(2026, 11, 1, 1, 30)), done(at(2026, 10, 31, 23)), done(at(2026, 10, 30, 12))]
      expect(computeStats(tasks, lists, now, 1).streak).toBe(4)
    })
  })

  it('ranks lists by completed tasks, ties by name, at most five, naming unknown lists Inbox', () => {
    const t = (listId: string, count: number): Task[] => Array.from({ length: count }, () => done(NOW, { listId }))
    const many = ['a', 'b', 'c', 'd', 'e', 'f'].flatMap((id) => t(id, 1))
    const s = computeStats([...t('home', 3), ...t('work', 3), ...t('gone', 1), ...many], lists, NOW, 1)
    expect(s.topLists.map((l) => [l.name, l.count])).toEqual([['Home', 3], ['Work', 3], ['Inbox', 1], ['Inbox', 1], ['Inbox', 1]])
    expect(s.topLists).toHaveLength(5)
  })
})

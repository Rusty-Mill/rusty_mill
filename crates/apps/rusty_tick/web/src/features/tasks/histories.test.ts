import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { addDays, atTime, startOfDay } from '@/lib/date'
import { completedGroups, dayLabel, groupByDay, inRange, trashGroups } from './histories'

const NOW = new Date(2026, 8, 29, 12).getTime() // Tue 2026-09-29
const day = (offset: number, h = 12) => atTime(addDays(startOfDay(NOW), offset), h)
let n = 0
const task = (o: Partial<Task>): Task => ({ id: `t${++n}`, listId: 'a', title: `t${n}`, status: 'done', completedMs: null, deletedMs: null, ...o }) as Task

describe('inRange', () => {
  it('today and yesterday are calendar days', () => {
    expect(inRange(day(0, 0), 'today', NOW, 1)).toBe(true)
    expect(inRange(day(0, 23), 'today', NOW, 1)).toBe(true)
    expect(inRange(day(-1, 23), 'today', NOW, 1)).toBe(false)
    expect(inRange(day(-1), 'yesterday', NOW, 1)).toBe(true)
    expect(inRange(day(-2), 'yesterday', NOW, 1)).toBe(false)
  })
  it('the week starts where the preference says', () => {
    // Tue 29th: a Monday-start week began the 28th, a Sunday-start one the 27th.
    expect(inRange(day(-1), 'week', NOW, 1)).toBe(true) // Mon 28
    expect(inRange(day(-2), 'week', NOW, 1)).toBe(false) // Sun 27
    expect(inRange(day(-2), 'week', NOW, 0)).toBe(true)
    expect(inRange(day(5), 'week', NOW, 1)).toBe(true) // Sun Oct 4 is still this Monday-start week
    expect(inRange(day(6), 'week', NOW, 1)).toBe(false)
  })
  it('the month is the calendar month', () => {
    expect(inRange(day(-28), 'month', NOW, 1)).toBe(true)
    expect(inRange(day(-29), 'month', NOW, 1)).toBe(false)
    expect(inRange(day(1), 'month', NOW, 1)).toBe(true)
    expect(inRange(day(2), 'month', NOW, 1)).toBe(false)
  })
  it('all is everything', () => expect(inRange(0, 'all', NOW, 1)).toBe(true))
})

describe('dayLabel', () => {
  it('names near days and dates the rest', () => {
    expect(dayLabel(startOfDay(NOW), NOW)).toBe('Today')
    expect(dayLabel(addDays(startOfDay(NOW), -1), NOW)).toBe('Yesterday')
    expect(dayLabel(addDays(startOfDay(NOW), -3), NOW)).toBe('Sat, Sep 26')
    expect(dayLabel(new Date(2025, 11, 31).getTime(), NOW)).toBe('Wed, Dec 31, 2025')
  })
})

describe('groupByDay', () => {
  it('newest day first, newest task first within a day', () => {
    const a = task({ completedMs: day(0, 9) })
    const b = task({ completedMs: day(0, 15) })
    const c = task({ completedMs: day(-1, 10) })
    const groups = groupByDay([a, c, b], (t) => t.completedMs, NOW)
    expect(groups.map((g) => [g.label, g.tasks.map((t) => t.id)])).toEqual([['Today', [b.id, a.id]], ['Yesterday', [c.id]]])
  })
  it('skips tasks with no timestamp', () => {
    expect(groupByDay([task({ completedMs: null })], (t) => t.completedMs, NOW)).toEqual([])
  })
  it('splits at local midnight', () => {
    const late = task({ completedMs: atTime(startOfDay(NOW), 23) + 59 * 60_000 })
    const early = task({ completedMs: atTime(addDays(startOfDay(NOW), 1), 0) })
    expect(groupByDay([late, early], (t) => t.completedMs, NOW)).toHaveLength(2)
  })
})

describe('completedGroups', () => {
  const tasks = [
    task({ completedMs: day(0), listId: 'a' }),
    task({ completedMs: day(-1), listId: 'b' }),
    task({ completedMs: day(-40), listId: 'a' }),
    task({ status: 'open', completedMs: null }),
    task({ completedMs: day(0), deletedMs: 1 }),
  ]
  it('takes only completed, live tasks', () => {
    const all = completedGroups(tasks, { range: 'all', listId: null }, NOW, 1)
    expect(all.flatMap((g) => g.tasks)).toHaveLength(3)
  })
  it('filters by date and by list', () => {
    expect(completedGroups(tasks, { range: 'today', listId: null }, NOW, 1).flatMap((g) => g.tasks)).toHaveLength(1)
    expect(completedGroups(tasks, { range: 'all', listId: 'a' }, NOW, 1).flatMap((g) => g.tasks)).toHaveLength(2)
    expect(completedGroups(tasks, { range: 'today', listId: 'b' }, NOW, 1)).toEqual([])
  })
})

describe('trashGroups', () => {
  it('groups deleted tasks by deletion day', () => {
    const groups = trashGroups([task({ deletedMs: day(0) }), task({ deletedMs: day(-2) }), task({ deletedMs: null })], NOW)
    expect(groups.map((g) => g.label)).toEqual(['Today', 'Sun, Sep 27'])
  })
})

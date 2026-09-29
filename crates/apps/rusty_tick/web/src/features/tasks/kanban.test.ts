import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { addDays, atTime, startOfDay } from '@/lib/date'
import { dropPatch, kanbanGroupBy } from './kanban'

const NOW = new Date(2026, 8, 29, 12).getTime()
const task = (o: Partial<Task> = {}): Task => ({
  id: 't', listId: 'a', parentId: null, title: 't', notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: null,
  isAllDay: false, timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: 0, createdMs: 0, updatedMs: 0,
  completedMs: null, deletedMs: null, etag: '1', ...o,
})

describe('kanbanGroupBy', () => {
  it('uses priority when the view has no grouping', () => {
    expect(kanbanGroupBy('none')).toBe('priority')
    expect(kanbanGroupBy('date')).toBe('date')
  })
})

describe('dropPatch', () => {
  it('does nothing for a drop on the same column', () => {
    expect(dropPatch(task(), 'priority', 'p5', 'p5', NOW)).toBeNull()
  })
  it('priority', () => {
    expect(dropPatch(task(), 'priority', 'p0', 'p5', NOW)).toEqual({ priority: 5 })
    expect(dropPatch(task(), 'priority', 'p5', 'p0', NOW)).toEqual({ priority: 0 })
    expect(dropPatch(task(), 'priority', 'p5', 'bogus', NOW)).toBeNull()
  })
  it('list', () => {
    expect(dropPatch(task(), 'list', 'a', 'b', NOW)).toEqual({ listId: 'b' })
  })
  it('tag: swaps the column tag, keeps the others, and untagging leaves none of the column', () => {
    const t = task({ tags: ['x', 'y'] })
    expect(dropPatch(t, 'tag', 'x', 'z', NOW)).toEqual({ tags: ['y', 'z'] })
    expect(dropPatch(t, 'tag', 'x', 'notag', NOW)).toEqual({ tags: ['y'] })
    expect(dropPatch(task(), 'tag', 'notag', 'x', NOW)).toEqual({ tags: ['x'] })
    expect(dropPatch(task({ tags: ['x', 'z'] }), 'tag', 'x', 'z', NOW)).toEqual({ tags: ['z'] }) // no duplicate
  })
  it('date: sets the day, keeping a time of day if there was one', () => {
    const today = startOfDay(NOW)
    expect(dropPatch(task(), 'date', 'nodate', 'today', NOW)).toEqual({ dueMs: today, isAllDay: true })
    expect(dropPatch(task(), 'date', 'nodate', 'tomorrow', NOW)).toEqual({ dueMs: addDays(today, 1), isAllDay: true })
    const timed = task({ dueMs: atTime(addDays(NOW, -3), 17, 30) })
    expect(dropPatch(timed, 'date', 'overdue', 'tomorrow', NOW)).toEqual({ dueMs: atTime(addDays(today, 1), 17, 30) })
    expect(dropPatch(timed, 'date', 'overdue', 'nodate', NOW)).toEqual({ dueMs: null, startMs: null, isAllDay: false })
  })
  it('date: Overdue is not a place a card can be dropped', () => {
    expect(dropPatch(task(), 'date', 'today', 'overdue', NOW)).toBeNull()
  })
})

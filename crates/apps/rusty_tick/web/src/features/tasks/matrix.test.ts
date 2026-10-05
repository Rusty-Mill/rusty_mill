import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { addDays, startOfDay } from '@/lib/date'
import { matrixDropPatch, quadrantOf, splitQuadrants } from './matrix'

const NOW = new Date(2026, 8, 29, 12).getTime()
const TODAY = startOfDay(NOW)
const task = (o: Partial<Task> = {}): Task => ({
  id: 't', listId: 'a', parentId: null, title: 't', notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: null,
  isAllDay: false, timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: 0, createdMs: 0, updatedMs: 0,
  completedMs: null, deletedMs: null, etag: '1', ...o,
})

describe('quadrantOf', () => {
  it('important means medium or high priority; urgent means due today or overdue', () => {
    expect(quadrantOf(task({ priority: 5, dueMs: TODAY, isAllDay: true }), NOW)).toBe('do')
    expect(quadrantOf(task({ priority: 3, dueMs: addDays(TODAY, -2), isAllDay: true }), NOW)).toBe('do')
    expect(quadrantOf(task({ priority: 5 }), NOW)).toBe('plan')
    expect(quadrantOf(task({ priority: 3, dueMs: addDays(TODAY, 3), isAllDay: true }), NOW)).toBe('plan')
    expect(quadrantOf(task({ priority: 1, dueMs: TODAY, isAllDay: true }), NOW)).toBe('delegate')
    expect(quadrantOf(task(), NOW)).toBe('drop')
  })
})

describe('splitQuadrants', () => {
  it('places every task once and keeps order', () => {
    const tasks = [task({ id: 'a' }), task({ id: 'b', priority: 5 }), task({ id: 'c' })]
    const q = splitQuadrants(tasks, NOW)
    expect(q.drop.map((t) => t.id)).toEqual(['a', 'c'])
    expect(q.plan.map((t) => t.id)).toEqual(['b'])
  })
})

describe('matrixDropPatch', () => {
  it('does nothing for the quadrant the task is already in', () => {
    expect(matrixDropPatch(task(), 'drop', NOW)).toBeNull()
  })
  it('importance only touches priority', () => {
    expect(matrixDropPatch(task(), 'plan', NOW)).toEqual({ priority: 5 })
    expect(matrixDropPatch(task({ priority: 3 }), 'drop', NOW)).toEqual({ priority: 0 })
  })
  it('urgency only touches the due date', () => {
    expect(matrixDropPatch(task(), 'delegate', NOW)).toEqual({ dueMs: TODAY, isAllDay: true })
    const urgent = task({ dueMs: TODAY, isAllDay: true })
    expect(matrixDropPatch(urgent, 'drop', NOW)).toEqual({ dueMs: addDays(TODAY, 3), isAllDay: true })
  })
  it('a diagonal move changes both axes', () => {
    expect(matrixDropPatch(task(), 'do', NOW)).toEqual({ priority: 5, dueMs: TODAY, isAllDay: true })
  })
})

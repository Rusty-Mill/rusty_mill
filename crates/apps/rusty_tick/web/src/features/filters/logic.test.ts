import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { asFilterBody, emptyRule, matchesFilter } from './logic'

const task = (o: Partial<Task> = {}): Task => ({
  id: 't', listId: 'a', parentId: null, title: 't', notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: null,
  isAllDay: false, timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: 0, createdMs: 0, updatedMs: 0,
  completedMs: null, deletedMs: null, etag: '1', ...o,
})

describe('matchesFilter', () => {
  it('an empty rule matches everything', () => {
    expect(matchesFilter(task(), emptyRule(), 'nodate')).toBe(true)
  })
  it('within a field any value matches; across fields all must', () => {
    const rule = { ...emptyRule(), tags: ['x', 'y'], priorities: [5 as const] }
    expect(matchesFilter(task({ tags: ['y'], priority: 5 }), rule, 'nodate')).toBe(true)
    expect(matchesFilter(task({ tags: ['y'], priority: 3 }), rule, 'nodate')).toBe(false)
    expect(matchesFilter(task({ tags: ['z'], priority: 5 }), rule, 'nodate')).toBe(false)
  })
  it('lists and date buckets', () => {
    const rule = { ...emptyRule(), lists: ['a'], dates: ['overdue' as const, 'today' as const] }
    expect(matchesFilter(task(), rule, 'today')).toBe(true)
    expect(matchesFilter(task(), rule, 'later')).toBe(false)
    expect(matchesFilter(task({ listId: 'b' }), rule, 'today')).toBe(false)
  })
})

describe('asFilterBody', () => {
  it('rejects non-filters and blank names', () => {
    expect(asFilterBody(null)).toBeNull()
    expect(asFilterBody({ name: ' ' })).toBeNull()
  })
  it('drops unknown rule values and tolerates a missing rule', () => {
    expect(asFilterBody({ name: 'f' })).toEqual({ name: 'f', rule: emptyRule() })
    expect(asFilterBody({ name: 'f', rule: { priorities: [5, 2], dates: ['today', 'bogus'], tags: ['a', 1] } })).toEqual({
      name: 'f',
      rule: { lists: [], tags: ['a'], priorities: [5], dates: ['today'] },
    })
  })
})

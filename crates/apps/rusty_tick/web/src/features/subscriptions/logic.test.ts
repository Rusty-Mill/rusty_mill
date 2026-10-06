import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import type { IcsTask } from '@/lib/ics'
import { asSubscriptionBody, isStale, MAX_ITEMS, planSync } from './logic'

const ev = (uid: string, o: Partial<IcsTask> = {}): IcsTask => ({ uid, title: uid, notes: '', startMs: null, dueMs: 1000, isAllDay: true, timeZone: '', repeatFlag: '', ...o })
const task = (id: string, o: Partial<Task> = {}): Task => ({
  id, listId: 'l', parentId: null, title: id, notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: 1000, isAllDay: true,
  timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: 0, createdMs: 0, updatedMs: 0, completedMs: null,
  deletedMs: null, etag: '1', ...o,
})

describe('planSync', () => {
  it('creates tasks for new events', () => {
    const plan = planSync([ev('a'), ev('b')], {}, {})
    expect(plan.create.map((e) => e.uid)).toEqual(['a', 'b'])
    expect(plan.items).toEqual({})
  })

  it('updates only the fields the feed owns, and only when they changed', () => {
    const tasks = { t1: task('t1', { title: 'old', priority: 5, tags: ['mine'] }), t2: task('t2', { title: 't2' }) }
    const plan = planSync([ev('a', { title: 'new', dueMs: 2000 }), ev('b', { title: 't2' })], { a: 't1', b: 't2' }, tasks)
    expect(plan.update).toEqual([{ taskId: 't1', patch: { title: 'new', dueMs: 2000 } }])
    expect(plan.items).toEqual({ a: 't1', b: 't2' })
  })

  it('trashes tasks whose event left the feed, and leaves ones already gone alone', () => {
    const tasks = { t1: task('t1'), t2: task('t2', { deletedMs: 5 }) }
    const plan = planSync([], { a: 't1', b: 't2', c: 'missing' }, tasks)
    expect(plan.trash).toEqual(['t1'])
    expect(plan.items).toEqual({})
  })

  it('does not bring back a task the user deleted', () => {
    const plan = planSync([ev('a', { title: 'new' })], { a: 't1' }, { t1: task('t1', { deletedMs: 5 }) })
    expect(plan.create).toEqual([])
    expect(plan.update).toEqual([])
    expect(plan.items).toEqual({ a: 't1' }) // still mapped, so it is not created next time either
  })

  it('ignores a repeated UID and caps the feed', () => {
    expect(planSync([ev('a'), ev('a', { title: 'again' })], {}, {}).create).toHaveLength(1)
    const many = Array.from({ length: MAX_ITEMS + 5 }, (_, i) => ev(`e${i}`))
    expect(planSync(many, {}, {}).create).toHaveLength(MAX_ITEMS)
  })
})

describe('bodies', () => {
  it('parses a stored subscription and refuses other documents', () => {
    expect(asSubscriptionBody({ name: 'Work', url: 'https://x/a.ics', listId: 'l', items: { a: 't', b: 5 } })).toEqual({
      name: 'Work', url: 'https://x/a.ics', listId: 'l', items: { a: 't' }, syncedMs: 0,
    })
    expect(asSubscriptionBody({ name: 'Work', url: '', listId: 'l' })).toBeNull()
    expect(asSubscriptionBody(null)).toBeNull()
  })
  it('knows when it is stale', () => {
    const s = { name: 'n', url: 'u', listId: 'l', items: {}, syncedMs: 1000 }
    expect(isStale(s, 5000, 3000)).toBe(true)
    expect(isStale(s, 3000, 3000)).toBe(false)
  })
})

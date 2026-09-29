import { describe, expect, it } from 'vitest'
import type { List, Tag, Task } from '@/api/types'
import { addDays, atTime, startOfDay } from '@/lib/date'
import {
  checklistProgress,
  defaultOptions,
  firstSortOrder,
  groupTasks,
  ORDER_STEP,
  renumber,
  sortOrderBetween,
  sortTasks,
  tasksForView,
  viewKey,
} from './organize'

const NOW = new Date(2026, 8, 29, 12).getTime() // Tue 2026-09-29 12:00
const INBOX = 'inbox'
let n = 0
const task = (over: Partial<Task> = {}): Task => ({
  id: `t${String(++n).padStart(3, '0')}`,
  listId: INBOX,
  parentId: null,
  title: 'task',
  notes: '',
  kind: 'text',
  status: 'open',
  priority: 0,
  startMs: null,
  dueMs: null,
  isAllDay: false,
  timeZone: '',
  reminders: [],
  repeatFlag: '',
  exDates: [],
  items: [],
  tags: [],
  sortOrder: n,
  createdMs: n,
  updatedMs: n,
  completedMs: null,
  deletedMs: null,
  etag: '1',
  ...over,
})
const list = (id: string, over: Partial<List> = {}): List => ({
  id,
  name: id,
  color: null,
  archived: false,
  viewMode: 'list',
  sortType: '',
  sortOrder: 0,
  updatedMs: 0,
  etag: '1',
  ...over,
})
const tag = (name: string, sortOrder = 0): Tag => ({ name, label: name.toUpperCase(), color: null, parent: null, sortOrder, etag: '1' })
const day = (offset: number, hour = 0) => atTime(addDays(startOfDay(NOW), offset), hour)
const ids = (ts: Task[]) => ts.map((t) => t.title)

describe('tasksForView', () => {
  const lists = [list(INBOX), list('work'), list('old', { archived: true })]
  const tasks = [
    task({ title: 'inbox-open' }),
    task({ title: 'work-open', listId: 'work', dueMs: day(0, 15) }),
    task({ title: 'old-open', listId: 'old', dueMs: day(0) }),
    task({ title: 'done', listId: 'work', status: 'done', dueMs: day(0) }),
    task({ title: 'trashed', listId: 'work', deletedMs: 1, dueMs: day(0) }),
    task({ title: 'overdue', listId: 'work', dueMs: day(-3) }),
    task({ title: 'in-3-days', listId: 'work', dueMs: day(3), tags: ['x'] }),
    task({ title: 'in-9-days', listId: 'work', dueMs: day(9) }),
  ]
  const e = { tasks, lists, tags: [], inboxId: INBOX }
  const titles = (spec: Parameters<typeof tasksForView>[0], done = false) => ids(tasksForView(spec, e, NOW, done)).sort()

  it('All: open, live tasks in lists that are not archived', () => {
    expect(titles({ kind: 'all' })).toEqual(['in-3-days', 'in-9-days', 'inbox-open', 'overdue', 'work-open'])
  })

  it('Today: due today or earlier', () => {
    expect(titles({ kind: 'today' })).toEqual(['overdue', 'work-open'])
  })

  it('Next 7 Days: from today, excluding overdue and beyond a week', () => {
    expect(titles({ kind: 'week' })).toEqual(['in-3-days', 'work-open'])
  })

  it('Inbox, list and tag views', () => {
    expect(titles({ kind: 'inbox' })).toEqual(['inbox-open'])
    expect(titles({ kind: 'list', id: 'old' })).toEqual(['old-open']) // an archived list still opens
    expect(titles({ kind: 'tag', name: 'x' })).toEqual(['in-3-days'])
  })

  it('can include completed tasks, never trashed ones', () => {
    expect(titles({ kind: 'list', id: 'work' }, true)).toContain('done')
    expect(titles({ kind: 'list', id: 'work' }, true)).not.toContain('trashed')
  })
})

describe('sortTasks', () => {
  const a = task({ title: 'b-task', dueMs: day(2), priority: 1, sortOrder: 30, createdMs: 3 })
  const b = task({ title: 'A-task', dueMs: day(1), priority: 5, sortOrder: 20, createdMs: 1 })
  const c = task({ title: 'c-task', dueMs: null, priority: 3, sortOrder: 10, createdMs: 2 })
  const all = [a, b, c]

  it('manual follows sortOrder', () => expect(ids(sortTasks(all, 'manual', 'asc'))).toEqual(['c-task', 'A-task', 'b-task']))
  it('date: soonest first, undated last', () => expect(ids(sortTasks(all, 'date', 'asc'))).toEqual(['A-task', 'b-task', 'c-task']))
  it('date descending flips the dated ones and still puts undated last', () => {
    expect(ids(sortTasks(all, 'date', 'desc'))).toEqual(['b-task', 'A-task', 'c-task'])
  })
  it('title is case-insensitive', () => expect(ids(sortTasks(all, 'title', 'asc'))).toEqual(['A-task', 'b-task', 'c-task']))
  it('title compares numbers as numbers', () => {
    const ts = [task({ title: 'item 10' }), task({ title: 'item 2' })]
    expect(ids(sortTasks(ts, 'title', 'asc'))).toEqual(['item 2', 'item 10'])
  })
  it('priority: highest first', () => expect(ids(sortTasks(all, 'priority', 'asc'))).toEqual(['A-task', 'c-task', 'b-task']))
  it('created: oldest first', () => expect(ids(sortTasks(all, 'created', 'asc'))).toEqual(['A-task', 'c-task', 'b-task']))
  it('ties fall back to manual order, so equal dates are deterministic', () => {
    const x = task({ title: 'x', dueMs: day(1), sortOrder: 2 })
    const y = task({ title: 'y', dueMs: day(1), sortOrder: 1 })
    expect(ids(sortTasks([x, y], 'date', 'asc'))).toEqual(['y', 'x'])
  })
  it('does not mutate its input', () => {
    const before = ids(all)
    sortTasks(all, 'title', 'desc')
    expect(ids(all)).toEqual(before)
  })
})

describe('groupTasks', () => {
  const ctx = { now: NOW, lists: [list('b', { sortOrder: 2 }), list('a', { sortOrder: 1 })], tags: [tag('x', 2), tag('y', 1)] }

  it('none is one unlabelled group', () => {
    const ts = [task(), task()]
    expect(groupTasks(ts, 'none', ctx)).toEqual([{ key: 'all', label: '', tasks: ts }])
  })

  it('date: the six buckets, in order, empty ones omitted', () => {
    const ts = [
      task({ title: 'later', dueMs: day(20) }),
      task({ title: 'none' }),
      task({ title: 'next7', dueMs: day(4) }),
      task({ title: 'tomorrow', dueMs: day(1, 9) }),
      task({ title: 'today-past-time', dueMs: day(0, 8) }),
      task({ title: 'today-all-day', dueMs: day(0), isAllDay: true }),
      task({ title: 'yesterday', dueMs: day(-1), isAllDay: true }),
    ]
    const groups = groupTasks(ts, 'date', ctx)
    expect(groups.map((g) => g.label)).toEqual(['Overdue', 'Today', 'Tomorrow', 'Next 7 Days', 'Later', 'No Date'])
    expect(groups[0]).toMatchObject({ tone: 'overdue' })
    expect(ids(groups[0]!.tasks)).toEqual(['today-past-time', 'yesterday'])
    expect(ids(groups[1]!.tasks)).toEqual(['today-all-day'])
    expect(groupTasks([task()], 'date', ctx).map((g) => g.label)).toEqual(['No Date'])
  })

  it('priority: high to none', () => {
    const ts = [task({ priority: 0 }), task({ priority: 5 }), task({ priority: 1 })]
    expect(groupTasks(ts, 'priority', ctx).map((g) => g.label)).toEqual(['High Priority', 'Low Priority', 'No Priority'])
  })

  it('list: in the lists\' own order', () => {
    const ts = [task({ listId: 'b' }), task({ listId: 'a' })]
    expect(groupTasks(ts, 'list', ctx).map((g) => g.key)).toEqual(['a', 'b'])
  })

  it('tag: a task with two tags appears under both; untagged ones get their own group', () => {
    const ts = [task({ title: 'both', tags: ['x', 'y'] }), task({ title: 'plain' })]
    const groups = groupTasks(ts, 'tag', ctx)
    expect(groups.map((g) => [g.label, ids(g.tasks)])).toEqual([
      ['Y', ['both']],
      ['X', ['both']],
      ['No Tag', ['plain']],
    ])
  })
})

describe('sort order for drag and drop', () => {
  it('sits between neighbours', () => {
    expect(sortOrderBetween(1000, 2000)).toBe(1500)
    expect(sortOrderBetween(0, 3)).toBe(1)
  })
  it('extends past either end', () => {
    expect(sortOrderBetween(null, 500)).toBe(500 - ORDER_STEP)
    expect(sortOrderBetween(500, null)).toBe(500 + ORDER_STEP)
    expect(sortOrderBetween(null, null)).toBe(0)
  })
  it('asks for a renumber when the gap has closed', () => {
    expect(sortOrderBetween(5, 6)).toBeNull()
    expect(sortOrderBetween(5, 5)).toBeNull()
    expect(renumber(3)).toEqual([0, ORDER_STEP, 2 * ORDER_STEP])
  })
  it('new tasks go first', () => {
    expect(firstSortOrder([])).toBe(0)
    expect(firstSortOrder([task({ sortOrder: 50 }), task({ sortOrder: -20 })])).toBe(-20 - ORDER_STEP)
  })
})

describe('small helpers', () => {
  it('view keys and defaults', () => {
    expect(viewKey({ kind: 'list', id: 'abc' })).toBe('list:abc')
    expect(viewKey({ kind: 'today' })).toBe('today')
    expect(defaultOptions({ kind: 'today' })).toMatchObject({ groupBy: 'date', sortBy: 'date' })
    expect(defaultOptions({ kind: 'list', id: 'x' })).toMatchObject({ groupBy: 'none', sortBy: 'manual' })
  })
  it('checklist progress', () => {
    expect(checklistProgress(task())).toBeNull()
    const items = [1, 2, 3].map((i) => ({ id: `i${i}`, title: 'x', done: i < 3, sortOrder: i }))
    expect(checklistProgress(task({ items }))).toBe('2/3')
  })
})

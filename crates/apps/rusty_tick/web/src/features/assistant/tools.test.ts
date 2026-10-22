import { describe, expect, it } from 'vitest'
import type { List, Task } from '@/api/types'
import { addDays, startOfDay } from '@/lib/date'
import { moveTo, overdue, pickTask, readDay, taskTools, type ToolContext } from './tools'

const NOW = new Date(2026, 9, 22, 12).getTime()
const task = (id: string, title: string, over: Partial<Task> = {}): Task =>
  ({ id, title, listId: 'inbox', status: 'open', dueMs: null, deletedMs: null, notes: '', tags: [], items: [], ...over }) as Task
const list = (id: string, name: string, archived = false): List => ({ id, name, archived }) as List

/** A context over `tasks` that records what the tools did. */
function context(tasks: Task[]): { c: ToolContext; done: string[] } {
  const done: string[] = []
  const c: ToolContext = {
    entities: { tasks, lists: [list('inbox', 'Inbox'), list('w', 'Work'), list('old', 'Old', true)], tags: [], inboxId: 'inbox', filters: [], assignees: {} },
    listId: 'inbox',
    now: NOW,
    createTask: async (input) => task('new', input.title, { listId: input.listId }),
    toggleDone: async (id) => void done.push(`toggle ${id}`),
    reopen: async (id) => void done.push(`reopen ${id}`),
    trash: async (id) => void done.push(`trash ${id}`),
    update: async (id, patch) => void done.push(`update ${id} ${JSON.stringify(patch)}`),
    move: async (id, listId) => void done.push(`move ${id} ${listId}`),
  }
  return { c, done }
}

const run = async (c: ToolContext, name: string, args: Record<string, string>): Promise<unknown> =>
  taskTools(c).find((t) => t.name === name)!.handler!(args)

describe('pickTask', () => {
  const tasks = [task('a', 'Buy milk'), task('b', 'Milk the cow'), task('c', 'milk')]

  it('prefers an exact title, then the only title containing the query', () => {
    expect(pickTask(tasks, ' MILK ', 'task')).toEqual({ task: tasks[2] })
    expect(pickTask(tasks, 'cow', 'task')).toEqual({ task: tasks[1] })
  })

  it('names no task for an empty or blank query', () => {
    for (const q of ['', '   ']) expect(pickTask(tasks, q, 'task')).toEqual({ error: 'Say which task.' })
  })

  it('names the candidates rather than guess, and says when nothing matches', () => {
    expect(pickTask(tasks.slice(0, 2), 'milk', 'task')).toEqual({ matches: ['Buy milk', 'Milk the cow'] })
    expect(pickTask(tasks, 'bread', 'open task')).toEqual({ error: 'No open task matches “bread”.' })
  })
})

describe('taskTools', () => {
  const yesterday = addDays(startOfDay(NOW), -1)
  const tasks = [
    task('late', 'Pay rent', { dueMs: yesterday }),
    task('today', 'Call mum', { dueMs: NOW }),
    task('later', 'Book flights', { dueMs: addDays(startOfDay(NOW), 3) }),
    task('closed', 'Buy milk', { status: 'done' }),
    task('binned', 'Old idea', { deletedMs: 1 }),
  ]

  it('adds to the open list', async () => {
    expect(await run(context(tasks).c, 'add_task', { title: 'Buy bread' })).toEqual({ title: 'Buy bread', id: 'new' })
  })

  it('lists what is due, overdue and found, and the live lists', async () => {
    const { c } = context(tasks)
    expect(overdue(tasks, NOW).map((t) => t.id)).toEqual(['late'])
    expect(await run(c, 'due_tasks', { view: 'overdue' })).toEqual({ tasks: [expect.stringMatching(/^Pay rent \(due /)] })
    expect(((await run(c, 'due_tasks', { view: 'today' })) as { tasks: string[] }).tasks).toHaveLength(2)
    expect(((await run(c, 'due_tasks', { view: 'next7' })) as { tasks: string[] }).tasks.map((l) => l.split(' (')[0])).toEqual(['Call mum', 'Book flights'])
    expect(await run(c, 'search_tasks', { query: 'flights' })).toEqual({ tasks: [expect.stringMatching(/^Book flights/)] })
    expect(await run(c, 'list_lists', {})).toEqual({ lists: ['Inbox', 'Work'] })
  })

  it('completes, reopens and trashes the one task named, among the right ones', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'complete_task', { query: 'rent' })).toEqual({ title: 'Pay rent' })
    expect(await run(c, 'complete_task', { query: 'buy milk' })).toEqual({ error: 'No open task matches “buy milk”.' })
    expect(await run(c, 'reopen_task', { query: 'buy milk' })).toEqual({ title: 'Buy milk' })
    expect(await run(c, 'delete_task', { query: 'old idea' })).toEqual({ error: 'No task matches “old idea”.' })
    expect(await run(c, 'delete_task', { query: 'call mum' })).toEqual({ title: 'Call mum' })
    expect(done).toEqual(['toggle late', 'reopen closed', 'trash today'])
  })
  it('changes nothing for an empty, blank or missing query, even with one task to pick', async () => {
    const { c, done } = context([task('only', 'Only task'), task('shut', 'Shut task', { status: 'done' })])
    for (const name of ['complete_task', 'reopen_task', 'delete_task']) {
      for (const args of [{ query: '' }, { query: '  ' }, {}] as Record<string, string>[]) expect(await run(c, name, args)).toEqual({ error: 'Say which task.' })
    }
    expect(done).toEqual([])
  })})

describe('readDay', () => {
  const day = (y: number, m: number, d: number): number => new Date(y, m - 1, d).getTime()

  it('reads the days people say, relative to now (a Thursday)', () => {
    expect(readDay('today', NOW)).toBe(day(2026, 10, 22))
    expect(readDay('Tomorrow', NOW)).toBe(day(2026, 10, 23))
    expect(readDay('friday', NOW)).toBe(day(2026, 10, 23))
    expect(readDay('on Mon', NOW)).toBe(day(2026, 10, 26))
    expect(readDay('thursday', NOW)).toBe(day(2026, 10, 29)) // today’s weekday means next week’s
    expect(readDay('in 3 days', NOW)).toBe(day(2026, 10, 25))
    expect(readDay('next week', NOW)).toBe(day(2026, 10, 29))
    expect(readDay('2026-11-05', NOW)).toBe(day(2026, 11, 5))
    expect(readDay('no date', NOW)).toBeNull()
  })

  it('reads nothing it is not sure of', () => {
    for (const text of ['soonish', '2026-02-30', '5/11', 'in many days', '']) expect(readDay(text, NOW)).toBeUndefined()
  })
})

describe('moveTo', () => {
  const friday = new Date(2026, 9, 23).getTime()

  it("keeps a timed task's time of day, and makes an undated one all-day", () => {
    const timed = task('t', 'Call', { dueMs: new Date(2026, 9, 20, 9, 30).getTime(), isAllDay: false, startMs: null })
    expect(moveTo(timed, friday)).toEqual({ dueMs: new Date(2026, 9, 23, 9, 30).getTime(), isAllDay: false })
    expect(moveTo(task('u', 'Idea', { startMs: null }), friday)).toEqual({ dueMs: friday, isAllDay: true })
  })

  it('drops a start that would follow the new due date, and clears both for no date', () => {
    const ranged = task('r', 'Trip', { startMs: new Date(2026, 9, 25).getTime(), dueMs: new Date(2026, 9, 27).getTime(), isAllDay: true })
    expect(moveTo(ranged, friday)).toEqual({ dueMs: friday, isAllDay: true, startMs: null })
    expect(moveTo(ranged, null)).toEqual({ dueMs: null, startMs: null })
  })
})

describe('update_task', () => {
  const tasks = [task('m', 'Buy milk', { startMs: null }), task('o', 'Buy oats', { startMs: null })]

  it('renames or moves the one task named', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', title: ' Buy oat milk ' })).toEqual({ title: 'Buy oat milk' })
    expect(await run(c, 'update_task', { query: 'oats', due: 'friday' })).toEqual({ title: 'Buy oats', due: expect.any(String) })
    expect(done).toEqual([
      'update m {"title":"Buy oat milk"}',
      `update o ${JSON.stringify({ dueMs: new Date(2026, 9, 23).getTime(), isAllDay: true })}`,
    ])
  })

  it('changes nothing for an unreadable day, a blank or missing change, or an unclear task', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', due: 'soonish' })).toEqual({ error: expect.stringContaining("can't read “soonish”") })
    expect(await run(c, 'update_task', { query: 'milk', title: '  ' })).toEqual({ error: 'Say the new title.' })
    expect(await run(c, 'update_task', { query: 'milk' })).toEqual({ error: 'Say what to change: a new title, a day, a priority or a list.' })
    expect(await run(c, 'update_task', { query: 'buy', due: 'today' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(await run(c, 'update_task', { query: ' ', due: 'today' })).toEqual({ error: 'Say which task.' })
    expect(done).toEqual([])
  })

  it('renames the task named exactly, leaving one whose title merely contains it', async () => {
    const { c, done } = context([task('m', 'Buy milk'), task('t', 'Buy milk to Talk')])
    expect(await run(c, 'update_task', { query: 'Buy milk', title: 'Talk to mum' })).toEqual({ title: 'Talk to mum' })
    expect(done).toEqual(['update m {"title":"Talk to mum"}'])
  })

  it('sets the priority of the one task named, and changes nothing for an unknown one', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', priority: 'high' })).toEqual({ title: 'Buy milk', priority: 'high' })
    expect(await run(c, 'update_task', { query: 'oats', priority: 'none' })).toEqual({ title: 'Buy oats', priority: 'none' })
    expect(await run(c, 'update_task', { query: 'milk', priority: 'urgent' })).toEqual({ error: 'Priority is high, medium, low or none.' })
    expect(await run(c, 'update_task', { query: 'buy', priority: 'low' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(done).toEqual(['update m {"priority":5}', 'update o {"priority":0}'])
  })

  it('moves the one task named to the one list named, and changes nothing otherwise', async () => {
    const { c, done } = context(tasks)
    expect(await run(c, 'update_task', { query: 'milk', list: 'work' })).toEqual({ title: 'Buy milk', list: 'Work' })
    expect(await run(c, 'update_task', { query: 'oats', list: 'inbox' })).toEqual({ title: 'Buy oats', list: 'Inbox' })
    expect(await run(c, 'update_task', { query: 'milk', list: 'Old' })).toEqual({ error: 'No list matches “Old”.' }) // archived
    expect(await run(c, 'update_task', { query: 'milk', list: 'o' })).toEqual({ error: expect.stringMatching(/^Several lists match “o”: Inbox, Work/) })
    expect(await run(c, 'update_task', { query: 'milk', list: ' ' })).toEqual({ error: 'Say which list.' })
    expect(await run(c, 'update_task', { query: 'buy', list: 'Work' })).toEqual({ matches: ['Buy milk', 'Buy oats'] })
    expect(done).toEqual(['move m w', 'move o inbox'])
  })
})

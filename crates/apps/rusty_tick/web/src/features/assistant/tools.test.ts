import { describe, expect, it } from 'vitest'
import type { List, Task } from '@/api/types'
import { addDays, startOfDay } from '@/lib/date'
import { overdue, pickTask, taskTools, type ToolContext } from './tools'

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
})

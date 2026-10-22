/**
 * The assistant's tools, as the browser runs them: named and shaped as the
 * MCP server's (`src/mcp.rs`), but given a title where MCP takes an id, since
 * a person names a task by what it says. Each answers with plain JSON the
 * agent turns into a reply; a task named ambiguously is not guessed at.
 */
import type { ActionDefinition } from '@rusty-mill/agui-core'
import type { NewTask, Priority, Task, TaskPatch } from '@/api/types'
import { searchTasks } from '@/features/search/search'
import { tasksForView, type Entities } from '@/features/tasks/organize'
import { addDays, startOfDay } from '@/lib/date'

/** Most tasks one answer lists. */
export const MAX_SHOWN = 20

type Answer = Record<string, string | string[]>

export interface ToolContext {
  /** Everything the store holds, trash included, for the views the tools read. */
  entities: Entities
  /** Where `add_task` adds: the list the person has open, else the Inbox. */
  listId: string
  now: number
  createTask(input: Omit<NewTask, 'id'>): Promise<Task>
  toggleDone(id: string): Promise<void>
  reopen(id: string): Promise<void>
  trash(id: string): Promise<void>
  update(id: string, patch: TaskPatch): Promise<void>
}

const norm = (text: string): string => text.trim().toLowerCase()

/** One line per task: its title and, if it has one, its due date. */
export const line = (t: Task): string => (t.dueMs === null ? t.title : `${t.title} (due ${new Date(t.dueMs).toLocaleDateString()})`)

export type Pick = { task: Task } | { matches: string[] } | { error: string }

/**
 * The one task among `tasks` that `query` names: its exact title, else the
 * only title containing it. A blank query names none (every title contains it).
 */
export function pickTask(tasks: readonly Task[], query: string, what: string): Pick {
  const q = norm(query)
  if (q === '') return { error: 'Say which task.' }
  const exact = tasks.filter((t) => norm(t.title) === q)
  const found = exact.length > 0 ? exact : tasks.filter((t) => norm(t.title).includes(q))
  if (found.length === 1) return { task: found[0]! }
  if (found.length === 0) return { error: `No ${what} matches “${query.trim()}”.` }
  return { matches: found.slice(0, 5).map((t) => t.title) }
}

/** Open tasks due before today. */
export const overdue = (tasks: readonly Task[], now: number): Task[] =>
  tasks.filter((t) => t.deletedMs === null && t.status === 'open' && t.dueMs !== null && t.dueMs < startOfDay(now))

const WEEKDAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday']
const NO_DATE = ['none', 'no date', 'no due date', 'someday']

/**
 * The day `text` names, as local midnight: today, tomorrow, next week, in N
 * days, a weekday (its next one, a week on if it is today), or YYYY-MM-DD.
 * `null` for no date; `undefined` when unreadable, so nothing is guessed.
 */
export function readDay(text: string, now: number): number | null | undefined {
  const t = norm(text).replace(/^(on|by|the) /, '')
  const today = startOfDay(now)
  if (NO_DATE.includes(t)) return null
  if (t === 'today') return today
  if (t === 'tomorrow') return addDays(today, 1)
  if (t === 'next week') return addDays(today, 7)
  const inDays = /^in (\d{1,3}) days?$/.exec(t)
  if (inDays) return addDays(today, Number(inDays[1]))
  const weekday = WEEKDAYS.findIndex((w) => t === w || t === w.slice(0, 3) || t === `next ${w}`)
  if (weekday >= 0) return addDays(today, (weekday - new Date(today).getDay() + 7) % 7 || 7)
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t)
  if (!iso) return undefined
  const [y, m, d] = [Number(iso[1]), Number(iso[2]) - 1, Number(iso[3])]
  const day = new Date(y, m, d)
  return day.getFullYear() === y && day.getMonth() === m && day.getDate() === d ? day.getTime() : undefined
}

/**
 * The patch that moves `t` to `day` (local midnight), or clears its dates for
 * `null`. A timed task keeps its time of day; an undated one becomes all-day;
 * a start that would fall after the new due date is dropped.
 */
export function moveTo(t: Task, day: number | null): TaskPatch {
  if (day === null) return { dueMs: null, startMs: null }
  const at = new Date(day)
  if (t.dueMs !== null && !t.isAllDay) {
    const old = new Date(t.dueMs)
    at.setHours(old.getHours(), old.getMinutes(), 0, 0)
  }
  const dueMs = at.getTime()
  const patch: TaskPatch = { dueMs, isAllDay: t.dueMs === null || t.isAllDay }
  if (t.startMs !== null && t.startMs > dueMs) patch.startMs = null
  return patch
}

const dayLabel = (ms: number): string => new Date(ms).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })

/** Priority words, as the agent sends them, and the priority each sets. */
const PRIORITY: Record<string, Priority> = { high: 5, medium: 3, med: 3, low: 1, none: 0, no: 0 }
const PRIORITY_NAME: Record<Priority, string> = { 5: 'high', 3: 'medium', 1: 'low', 0: 'none' }

/**
 * Rename the task `query` names, move it to the day `due` names, or set its
 * `priority`; every argument is checked before the task is picked, so a bad
 * one changes nothing.
 */
async function updateOne(c: ToolContext, tasks: readonly Task[], args: { [key: string]: string }): Promise<Answer> {
  const { query = '', title, due, priority } = args
  if (title === undefined && due === undefined && priority === undefined) return { error: 'Say what to change: a new title, a day or a priority.' }
  if (title !== undefined && title.trim() === '') return { error: 'Say the new title.' }
  const day = due === undefined ? undefined : readDay(due, c.now)
  if (due !== undefined && day === undefined) {
    return { error: `I can't read “${due.trim()}” as a day; try today, tomorrow, friday, in 3 days, next week, 2026-11-05 or none.` }
  }
  const level = priority === undefined ? undefined : PRIORITY[norm(priority)]
  if (priority !== undefined && level === undefined) return { error: 'Priority is high, medium, low or none.' }
  const pick = pickTask(tasks, query, 'task')
  if (!('task' in pick)) return pick
  const { task } = pick
  if (title !== undefined) {
    await c.update(task.id, { title: title.trim() })
    return { title: title.trim() }
  }
  if (level !== undefined) {
    await c.update(task.id, { priority: level })
    return { title: task.title, priority: PRIORITY_NAME[level] }
  }
  const patch = moveTo(task, day ?? null)
  await c.update(task.id, patch)
  return { title: task.title, due: patch.dueMs == null ? 'no date' : dayLabel(patch.dueMs) }
}

const titled = { type: 'object', properties: { query: { type: 'string', description: 'The task’s title, or enough of it' } }, required: ['query'] }

/** Run `act` on the one task `query` names among `tasks`, answering with its title. */
async function onOne(tasks: readonly Task[], query: string, what: string, act: (t: Task) => Promise<void>): Promise<Answer> {
  const pick = pickTask(tasks, query, what)
  if (!('task' in pick)) return pick
  await act(pick.task)
  return { title: pick.task.title }
}

/** The tools, bound to `c`; register each with `useAction`. */
export function taskTools(c: ToolContext): ActionDefinition<{ [key: string]: string }, Answer>[] {
  const live = c.entities.tasks.filter((t) => t.deletedMs === null)
  const lines = (tasks: Task[]): Answer => ({ tasks: tasks.slice(0, MAX_SHOWN).map(line) })
  return [
    {
      name: 'add_task',
      description: 'Add a task with this title to the list the person has open.',
      parameters: { type: 'object', properties: { title: { type: 'string' } }, required: ['title'] },
      handler: async ({ title }) => {
        const task = await c.createTask({ listId: c.listId, title: title ?? '' })
        return { title: task.title, id: task.id }
      },
    },
    {
      name: 'search_tasks',
      description: 'Tasks whose title or notes match the query.',
      parameters: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] },
      handler: ({ query }) => lines(searchTasks(query ?? '', c.entities.tasks, MAX_SHOWN).map((h) => h.task)),
    },
    {
      name: 'due_tasks',
      description: 'Open tasks due today (overdue included), in the next 7 days, or overdue.',
      parameters: { type: 'object', properties: { view: { type: 'string', enum: ['today', 'next7', 'overdue'] } }, required: ['view'] },
      handler: ({ view }) => {
        if (view === 'overdue') return lines(overdue(live, c.now))
        return lines(tasksForView({ kind: view === 'next7' ? 'week' : 'today' }, c.entities, c.now))
      },
    },
    {
      name: 'complete_task',
      description: 'Complete the one open task this names.',
      parameters: titled,
      handler: ({ query }) => onOne(live.filter((t) => t.status === 'open'), query ?? '', 'open task', (t) => c.toggleDone(t.id)),
    },
    {
      name: 'reopen_task',
      description: 'Open again the one done task this names.',
      parameters: titled,
      handler: ({ query }) => onOne(live.filter((t) => t.status !== 'open'), query ?? '', 'done task', (t) => c.reopen(t.id)),
    },
    {
      name: 'update_task',
      description: 'Rename the one task this names, move it to another day (today, tomorrow, friday, in 3 days, next week, 2026-11-05, or none), or set its priority.',
      parameters: {
        type: 'object',
        properties: {
          query: titled.properties.query,
          title: { type: 'string', description: 'The new title' },
          due: { type: 'string', description: 'The new day, as said' },
          priority: { type: 'string', enum: ['high', 'medium', 'low', 'none'] },
        },
        required: ['query'],
      },
      handler: (args) => updateOne(c, live, args),
    },
    {
      name: 'delete_task',
      description: 'Move the one task this names to the Trash.',
      parameters: titled,
      handler: ({ query }) => onOne(live, query ?? '', 'task', (t) => c.trash(t.id)),
    },
    {
      name: 'list_lists',
      description: 'The person’s lists, by name.',
      parameters: { type: 'object' },
      handler: () => ({ lists: c.entities.lists.filter((l) => !l.archived).map((l) => l.name) }),
    },
  ]
}

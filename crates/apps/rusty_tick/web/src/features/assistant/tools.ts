/**
 * The assistant's tools, as the browser runs them: named and shaped as the
 * MCP server's (`src/mcp.rs`), but given a title where MCP takes an id, since
 * a person names a task by what it says. Each answers with plain JSON the
 * agent turns into a reply; a task named ambiguously is not guessed at.
 */
import type { ActionDefinition } from '@rusty-mill/agui-core'
import type { List, NewTask, Priority, Task, TaskPatch } from '@/api/types'
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
  /** Move a task (and its subtasks) to another list. */
  move(id: string, listId: string): Promise<void>
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

/** A time of day, or none (`all-day`). */
export type Clock = { h: number; m: number } | 'all-day'

/** What a "move … to …" names: a day, a time of day, or both; a missing part is kept. */
export interface When {
  day?: number
  time?: Clock
}

const CLOCK = String.raw`\d{1,2}(?::\d{2})?(?:\s*(?:am|pm))?|noon|midnight|all day`

/**
 * `text` as a time of day: 3pm, 3:30 pm, 15:30, noon, midnight, all day. A
 * bare hour ("9") counts only after "at" (`bare`), so "friday 3" is not taken
 * for 3 o'clock; an hour or minute out of range is not read at all.
 */
function readClock(text: string, bare: boolean): Clock | undefined {
  const t = norm(text)
  if (t === 'all day') return 'all-day'
  if (t === 'noon') return { h: 12, m: 0 }
  if (t === 'midnight') return { h: 0, m: 0 }
  const parts = /^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/.exec(t)
  if (!parts || (!bare && parts[2] === undefined && parts[3] === undefined)) return undefined
  const [hour, minute, half] = [Number(parts[1]), Number(parts[2] ?? 0), parts[3]]
  if (minute > 59) return undefined
  if (half === undefined) return hour <= 23 ? { h: hour, m: minute } : undefined
  if (hour < 1 || hour > 12) return undefined
  return { h: (hour % 12) + (half === 'pm' ? 12 : 0), m: minute }
}

/**
 * The day and time `text` names, as `readDay` reads days, optionally followed
 * by a time ("friday at 3pm", "tomorrow 9:30", "2026-11-05 15:00"), or a time
 * alone ("3pm", "at 9", "all day"). `null` for no date; `undefined` when any
 * part is unreadable, so nothing is guessed.
 */
export function readWhen(text: string, now: number): When | null | undefined {
  const t = norm(text)
  const day = readDay(t, now)
  if (day !== undefined) return day === null ? null : { day }
  const atTime = /^(.*\S)\s+at\s+(.+)$/.exec(t) ?? /^()at\s+(.+)$/.exec(t)
  const split = atTime ?? new RegExp(`^(.*\\S)\\s+(${CLOCK})$`).exec(t) ?? new RegExp(`^()(${CLOCK})$`).exec(t)
  if (!split) return undefined
  const time = readClock(split[2]!, atTime !== null)
  if (time === undefined) return undefined
  if (split[1] === '') return { time }
  const on = readDay(split[1]!, now)
  return on === undefined || on === null ? undefined : { day: on, time }
}

/**
 * The patch that moves `t` to `day` (local midnight), at `time` if given, or
 * clears its dates for `null`. Without a time, a timed task keeps its time of
 * day and an undated one becomes all-day; a start that would fall after the
 * new due date is dropped.
 */
export function moveTo(t: Task, day: number | null, time?: Clock): TaskPatch {
  if (day === null) return { dueMs: null, startMs: null }
  const at = new Date(day)
  const keep = time === undefined && t.dueMs !== null && !t.isAllDay ? new Date(t.dueMs) : null
  if (time !== undefined && time !== 'all-day') at.setHours(time.h, time.m, 0, 0)
  if (keep) at.setHours(keep.getHours(), keep.getMinutes(), 0, 0)
  const dueMs = at.getTime()
  const isAllDay = time === undefined ? keep === null : time === 'all-day'
  const patch: TaskPatch = { dueMs, isAllDay }
  if (t.startMs !== null && t.startMs > dueMs) patch.startMs = null
  return patch
}

const dayLabel = (ms: number): string => new Date(ms).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })
const whenLabel = (patch: TaskPatch): string =>
  patch.dueMs == null ? 'no date' : patch.isAllDay ? dayLabel(patch.dueMs) : `${dayLabel(patch.dueMs)} at ${new Date(patch.dueMs).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`

/** The one live list `name` names: its exact name, else the only one containing it. */
export function pickList(lists: readonly List[], name: string): { list: List } | { error: string } {
  const q = norm(name)
  if (q === '') return { error: 'Say which list.' }
  const live = lists.filter((l) => !l.archived)
  const exact = live.filter((l) => norm(l.name) === q)
  const found = exact.length > 0 ? exact : live.filter((l) => norm(l.name).includes(q))
  if (found.length === 1) return { list: found[0]! }
  if (found.length === 0) return { error: `No list matches “${name.trim()}”.` }
  return { error: `Several lists match “${name.trim()}”: ${found.slice(0, 5).map((l) => l.name).join(', ')}. Say a little more of the one you mean.` }
}

/** Priority words, as the agent sends them, and the priority each sets. */
const PRIORITY: Record<string, Priority> = { high: 5, medium: 3, med: 3, low: 1, none: 0, no: 0 }
const PRIORITY_NAME: Record<Priority, string> = { 5: 'high', 3: 'medium', 1: 'low', 0: 'none' }

/**
 * Rename the task `query` names, move it to the day `due` names or the
 * `list` named, or set its `priority`; every argument is checked before the
 * task is picked, so a bad one changes nothing.
 */
async function updateOne(c: ToolContext, tasks: readonly Task[], args: { [key: string]: string }): Promise<Answer> {
  const { query = '', title, due, priority, list } = args
  if ([title, due, priority, list].every((v) => v === undefined)) return { error: 'Say what to change: a new title, a day, a priority or a list.' }
  if (title !== undefined && title.trim() === '') return { error: 'Say the new title.' }
  const when = due === undefined ? undefined : readWhen(due, c.now)
  if (due !== undefined && when === undefined) {
    return {
      error: `I can't read “${due.trim()}” as a day or time; try today, tomorrow, friday, in 3 days, next week, 2026-11-05, 3pm, friday at 9:30, all day or none.`,
    }
  }
  const level = priority === undefined ? undefined : PRIORITY[norm(priority)]
  if (priority !== undefined && level === undefined) return { error: 'Priority is high, medium, low or none.' }
  const target = list === undefined ? undefined : pickList(c.entities.lists, list)
  if (target !== undefined && 'error' in target) return target
  const pick = pickTask(tasks, query, 'task')
  if (!('task' in pick)) return pick
  const { task } = pick
  if (title !== undefined) {
    await c.update(task.id, { title: title.trim() })
    return { title: title.trim() }
  }
  if (target !== undefined) {
    await c.move(task.id, target.list.id)
    return { title: task.title, list: target.list.name }
  }
  if (level !== undefined) {
    await c.update(task.id, { priority: level })
    return { title: task.title, priority: PRIORITY_NAME[level] }
  }
  if (when === undefined) return { error: 'Say what to change: a new title, a day, a priority or a list.' }
  // A time alone keeps the task's day (today for an undated task).
  const patch = when === null ? moveTo(task, null) : moveTo(task, when.day ?? startOfDay(task.dueMs ?? c.now), when.time)
  await c.update(task.id, patch)
  return { title: task.title, due: whenLabel(patch) }
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
      description: 'Rename the one task this names, move it to another day or time (today, tomorrow, friday at 3pm, in 3 days, 2026-11-05 9:30, 3pm, all day, or none), set its priority, or move it to another list.',
      parameters: {
        type: 'object',
        properties: {
          query: titled.properties.query,
          title: { type: 'string', description: 'The new title' },
          due: { type: 'string', description: 'The new day and/or time, as said' },
          priority: { type: 'string', enum: ['high', 'medium', 'low', 'none'] },
          list: { type: 'string', description: 'The list to move it to, by name' },
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

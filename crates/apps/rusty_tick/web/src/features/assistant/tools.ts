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
import { describeRule, formatRule, parseRule, type Freq, type Rule } from '@/lib/recurrence'

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

const FREQ: Record<string, Freq> = { daily: 'DAILY', weekly: 'WEEKLY', monthly: 'MONTHLY', yearly: 'YEARLY', annually: 'YEARLY' }
const UNIT: Record<string, Freq> = { day: 'DAILY', week: 'WEEKLY', month: 'MONTHLY', year: 'YEARLY' }
const NEVER = ['never', 'none', 'no', 'not']

/** The weekday (`Date#getDay()`) a word names: monday, mon, mondays. */
const weekday = (word: string): number => WEEKDAYS.findIndex((w) => [w, `${w}s`, w.slice(0, 3)].includes(word))

/**
 * How often `text` says a task repeats, as the `RRULE` the app stores: daily,
 * weekly, monthly, yearly (annually), every N days/weeks/months/years, on
 * weekdays, or every monday[, wednesday and friday]. `''` for never;
 * `undefined` when unreadable, so nothing is guessed. A monthly rule keeps the
 * day of month of `due`, as the date picker's does, so the 31st stays the 31st
 * after a shorter month.
 */
export function readRepeat(text: string, due: number | null = null): string | undefined {
  const t = norm(text)
  if (NEVER.includes(t)) return ''
  const rule = (freq: Freq, interval = 1, byDay: number[] = []): string | undefined => {
    const byMonthDay = freq === 'MONTHLY' && due !== null ? new Date(due).getDate() : null
    const r: Rule = { freq, interval, byDay, byMonthDay, until: null }
    return interval >= 1 && interval <= 99 && parseRule(formatRule(r)) ? formatRule(r) : undefined
  }
  if (FREQ[t]) return rule(FREQ[t]!)
  if (/^(on |every )?weekdays?$/.test(t)) return rule('WEEKLY', 1, [1, 2, 3, 4, 5])
  const every = /^every (?:(\d{1,3}) )?(day|week|month|year)s?$/.exec(t)
  if (every) return rule(UNIT[every[2]!]!, Number(every[1] ?? 1))
  const days = /^(?:every|on) (.+)$/.exec(t)?.[1]?.split(/\s*(?:,|\band\b)\s*/).filter(Boolean)
  if (!days || days.length === 0) return undefined
  const byDay = days.map(weekday)
  return byDay.includes(-1) ? undefined : rule('WEEKLY', 1, [...new Set(byDay)].sort((a, b) => a - b))
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

/** A reminder as said: `n` units before the due time (0: at it), or none. */
export type Remind = 'none' | { n: number; unit: 'minute' | 'hour' | 'day' }
const REMIND_UNIT: Record<string, { unit: 'minute' | 'hour' | 'day'; max: number }> = {
  minute: { unit: 'minute', max: 1440 },
  min: { unit: 'minute', max: 1440 },
  hour: { unit: 'hour', max: 72 },
  hr: { unit: 'hour', max: 72 },
  day: { unit: 'day', max: 30 },
}
const REMIND_AT = ['at the time', 'at time', 'on time', 'on the day']

/** When `text` says to remind: "1 hour before", "a day before", "at the time", or none. */
export function readRemind(text: string): Remind | undefined {
  const t = norm(text)
  if (NEVER.includes(t) || t === 'off') return 'none'
  if (REMIND_AT.includes(t)) return { n: 0, unit: 'day' }
  const m = /^(\d{1,4}|a|an|one) (minute|min|hour|hr|day)s? before$/.exec(t)
  const unit = m ? REMIND_UNIT[m[2]!] : undefined
  if (!m || !unit) return undefined
  const n = /^\d/.test(m[1]!) ? Number(m[1]) : 1
  return n <= unit.max ? { n, unit: unit.unit } : undefined
}

/**
 * The `TRIGGER:` the app stores for `r` on a task, as the date picker writes
 * them: from the due time, or from 09:00 on the day for an all-day task, which
 * has no time to count minutes or hours from (`undefined`).
 */
export function remindTrigger(r: { n: number; unit: 'minute' | 'hour' | 'day' }, allDay: boolean): string | undefined {
  if (allDay) {
    if (r.n === 0) return 'TRIGGER:PT9H'
    return r.unit === 'day' ? `TRIGGER:-PT${24 * r.n - 9}H` : undefined
  }
  if (r.n === 0) return 'TRIGGER:PT0S'
  return { minute: `TRIGGER:-PT${r.n}M`, hour: `TRIGGER:-PT${r.n}H`, day: `TRIGGER:-P${r.n}D` }[r.unit]
}

const remindLabel = (r: { n: number; unit: string }, allDay: boolean): string => {
  const when = r.n === 0 ? (allDay ? 'on the day' : 'at the time') : `${r.n} ${r.unit}${r.n === 1 ? '' : 's'} before`
  return allDay ? `${when}, at 09:00` : when
}

/** Remind about `task` as `r` says: the patch, or why it cannot be. */
function remindPatch(task: Task, r: Remind): { patch: TaskPatch; said: string } | { error: string } {
  if (r === 'none') return { patch: { reminders: [] }, said: 'none' }
  if (task.dueMs === null) return { error: `“${task.title}” has no date to remind you from; give it one first, as in “move ${task.title} to friday at 3pm”.` }
  const trigger = remindTrigger(r, task.isAllDay)
  if (trigger === undefined) return { error: `“${task.title}” is all day, so it can only remind you on the day or days before (at 09:00); give it a time to remind you minutes or hours before.` }
  // One reminder, as the date picker keeps: it shows and edits only the first.
  return { patch: { reminders: [trigger] }, said: remindLabel(r, task.isAllDay) }
}

/** Priority words, as the agent sends them, and the priority each sets. */
const PRIORITY: Record<string, Priority> = { high: 5, medium: 3, med: 3, low: 1, none: 0, no: 0 }
const PRIORITY_NAME: Record<Priority, string> = { 5: 'high', 3: 'medium', 1: 'low', 0: 'none' }

/**
 * Rename the task `query` names, move it to the day `due` names or the
 * `list` named, or set its `priority`, `repeat` or `remind`; every argument is checked
 * before the task is picked, so a bad one changes nothing.
 */
async function updateOne(c: ToolContext, tasks: readonly Task[], args: { [key: string]: string }): Promise<Answer> {
  const { query = '', title, due, priority, list, repeat, remind } = args
  if ([title, due, priority, list, repeat, remind].every((v) => v === undefined)) return { error: 'Say what to change: a new title, a day, a priority, a list, a repeat or a reminder.' }
  if (title !== undefined && title.trim() === '') return { error: 'Say the new title.' }
  const when = due === undefined ? undefined : readWhen(due, c.now)
  if (due !== undefined && when === undefined) {
    return {
      error: `I can't read “${due.trim()}” as a day or time; try today, tomorrow, friday, in 3 days, next week, 2026-11-05, 3pm, friday at 9:30, all day or none.`,
    }
  }
  const level = priority === undefined ? undefined : PRIORITY[norm(priority)]
  if (priority !== undefined && level === undefined) return { error: 'Priority is high, medium, low or none.' }
  const rule = repeat === undefined ? undefined : readRepeat(repeat)
  if (repeat !== undefined && rule === undefined) {
    return { error: `I can't read “${repeat.trim()}” as a repeat; try daily, weekly, every 2 weeks, on weekdays, every monday and friday, monthly or never.` }
  }
  const reminder = remind === undefined ? undefined : readRemind(remind)
  if (remind !== undefined && reminder === undefined) {
    return { error: `I can't read “${remind.trim()}” as a reminder; try at the time, 5 minutes before, 1 hour before, a day before or none.` }
  }
  const target = list === undefined ? undefined : pickList(c.entities.lists, list)
  if (target !== undefined && 'error' in target) return target
  const pick = pickTask(tasks, query, 'task')
  if (!('task' in pick)) return pick
  const { task } = pick
  if (title !== undefined) {
    await c.update(task.id, { title: title.trim() })
    return { title: title.trim() }
  }
  if (reminder !== undefined) {
    const made = remindPatch(task, reminder)
    if ('error' in made) return made
    await c.update(task.id, made.patch)
    return { title: task.title, remind: made.said }
  }
  if (repeat !== undefined && rule !== undefined) {
    // A repeat needs a date to count from: an undated task starts today, as in the date picker.
    const due = task.dueMs ?? startOfDay(c.now)
    const anchored = readRepeat(repeat, due) ?? rule // the same words, now anchored on the due day
    const patch: TaskPatch = anchored !== '' && task.dueMs === null ? { repeatFlag: anchored, dueMs: due, isAllDay: true } : { repeatFlag: anchored }
    await c.update(task.id, patch)
    const said = anchored === '' ? 'never' : describeRule(anchored).replace(/^Every/, 'every')
    return patch.dueMs === undefined ? { title: task.title, repeat: said } : { title: task.title, repeat: said, due: whenLabel(patch) }
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
      description: 'Rename the one task this names, move it to another day or time (today, tomorrow, friday at 3pm, in 3 days, 2026-11-05 9:30, 3pm, all day, or none), set its priority, repeat or reminder, or move it to another list.',
      parameters: {
        type: 'object',
        properties: {
          query: titled.properties.query,
          title: { type: 'string', description: 'The new title' },
          due: { type: 'string', description: 'The new day and/or time, as said' },
          priority: { type: 'string', enum: ['high', 'medium', 'low', 'none'] },
          list: { type: 'string', description: 'The list to move it to, by name' },
          repeat: { type: 'string', description: 'How often it repeats, as said (daily, every 2 weeks, on weekdays, never)' },
          remind: { type: 'string', description: 'When to remind, as said (at the time, 1 hour before, a day before, none)' },
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

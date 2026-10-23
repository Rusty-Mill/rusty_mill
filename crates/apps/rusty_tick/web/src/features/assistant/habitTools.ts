/**
 * The assistant's habit tools, run in the browser against the habits store.
 * Habits are client documents with no API route, so unlike the task tools
 * these have no MCP twin. A habit named ambiguously is not guessed at.
 */
import type { ActionDefinition } from '@rusty-mill/agui-core'
import { checkinId, formatGoal, isDue, parseGoal, type Checkin, type Habit } from '@/features/habits/logic'
import type { HabitInput } from '@/features/habits/store'
import { SWATCHES } from '@/lib/colors'
import { dayKey, startOfDay } from '@/lib/date'

type Answer = Record<string, string | string[]>

export interface HabitContext {
  habits: readonly Habit[]
  checkins: Readonly<Record<string, Checkin>>
  now: number
  add(input: HabitInput): string
  /** Set a day's check-ins to `count` (0 clears the day). */
  setCount(habitId: string, day: string, count: number): void
}

const norm = (text: string): string => text.trim().toLowerCase()

/** The one live habit `query` names: its exact name, else the only name containing it. */
export function pickHabit(habits: readonly Habit[], query: string): { habit: Habit } | { matches: string[] } | { error: string } {
  const q = norm(query)
  if (q === '') return { error: 'Say which habit.' }
  const live = habits.filter((h) => !h.archived)
  const exact = live.filter((h) => norm(h.name) === q)
  const found = exact.length > 0 ? exact : live.filter((h) => norm(h.name).includes(q))
  if (found.length === 1) return { habit: found[0]! }
  if (found.length === 0) return { error: `No habit matches “${query.trim()}”.` }
  return { matches: found.slice(0, 5).map((h) => h.name) }
}

/** Today's check-ins of `h` against its daily goal. */
function progress(c: HabitContext, h: Habit, day: string): { count: number; goal: number } {
  return { count: c.checkins[checkinId(h.id, day)]?.count ?? 0, goal: parseGoal(h.goal).count }
}

/**
 * One more check-in today for the habit `query` names, up to its goal (one
 * past it is not a clear, as a click on the page would be), or none (`undo`).
 */
function checkIn(c: HabitContext, query: string, undo: boolean): Answer {
  const pick = pickHabit(c.habits, query)
  if (!('habit' in pick)) return pick
  const { habit } = pick
  const day = dayKey(startOfDay(c.now))
  const { count, goal } = progress(c, habit, day)
  if (undo) {
    c.setCount(habit.id, day, 0)
    return { title: habit.name, undone: 'yes' }
  }
  if (count >= goal) return { title: habit.name, progress: `${count}/${goal}`, already: 'yes' }
  c.setCount(habit.id, day, count + 1)
  return { title: habit.name, progress: `${count + 1}/${goal}` }
}

function addHabit(c: HabitContext, name: string): Answer {
  const title = name.trim()
  if (title === '') return { error: 'Say the habit’s name.' }
  if (c.habits.some((h) => !h.archived && norm(h.name) === norm(title))) return { error: `You already have a habit called “${title}”.` }
  c.add({ name: title, color: SWATCHES[5], goal: formatGoal(1, 'time'), frequency: { kind: 'daily' }, reminder: null })
  return { title }
}

/** The habit tools, bound to `c`; register each with `useAction`. */
export function habitTools(c: HabitContext): ActionDefinition<{ [key: string]: string }, Answer>[] {
  const named = { query: { type: 'string', description: 'The habit’s name, or enough of it' } }
  return [
    {
      name: 'list_habits',
      description: 'The habits due today, with today’s check-ins against each goal.',
      parameters: { type: 'object' },
      handler: () => {
        const day = dayKey(startOfDay(c.now))
        const due = c.habits.filter((h) => !h.archived && isDue(h, startOfDay(c.now)))
        return {
          habits: due.map((h) => {
            const { count, goal } = progress(c, h, day)
            return `${h.name} (${count}/${goal})`
          }),
        }
      },
    },
    {
      name: 'check_in_habit',
      description: 'Check in the one habit this names for today, once more up to its goal; or clear today’s check-ins (undo).',
      parameters: { type: 'object', properties: { ...named, undo: { type: 'string', enum: ['true'] } }, required: ['query'] },
      handler: ({ query, undo }) => checkIn(c, query ?? '', undo === 'true'),
    },
    {
      name: 'add_habit',
      description: 'Add a daily habit, once a day, with this name.',
      parameters: { type: 'object', properties: { name: { type: 'string' } }, required: ['name'] },
      handler: ({ name }) => addHabit(c, name ?? ''),
    },
  ]
}

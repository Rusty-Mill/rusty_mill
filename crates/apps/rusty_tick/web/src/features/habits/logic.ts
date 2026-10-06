/** Pure habit rules: due days, streaks, completion, week ranges, deterministic ids. No clock reads. */
import { derivedId } from '@/lib/id'
import { addDays, dayKey, parseDayKey, startOfDay, startOfWeek, type WeekStart } from '@/lib/date'

export type Frequency = { kind: 'daily' } | { kind: 'weekdays'; days: number[] } | { kind: 'perWeek'; times: number }

export interface HabitBody {
  name: string
  color: string
  /** Free text such as "1 time per day" or "8 glasses per day". */
  goal: string
  frequency: Frequency
  /** `HH:MM`, or null. Fired as a browser notification when the user has turned notifications on. */
  reminder: string | null
  createdMs: number
  archived: boolean
}

export interface Habit extends HabitBody {
  id: string
}

export interface Checkin {
  id: string
  habitId: string
  day: string
  count: number
}

export interface Streak {
  current: number
  best: number
  /** Daily and weekday habits count days; N-per-week habits count weeks. */
  unit: 'day' | 'week'
}

// ---- ids ----------------------------------------------------------------

/**
 * The id of the check-in doc for a habit on a day. Every device derives the
 * same id, so checking in twice is one upsert and unchecking is one delete.
 */
export function checkinId(habitId: string, day: string): string {
  return derivedId(`${habitId.toLowerCase()}|${day}`)
}

// ---- weeks and due days ----------------------------------------------------

/** The seven local midnights of the week containing `ms`. */
export function weekDays(ms: number, weekStart: WeekStart): number[] {
  const first = startOfWeek(ms, weekStart)
  return Array.from({ length: 7 }, (_, i) => addDays(first, i))
}

/** Weekday indexes (0 = Sunday) in the order a week starting on `weekStart` shows them. */
export function weekdayOrder(weekStart: WeekStart): number[] {
  return Array.from({ length: 7 }, (_, i) => (weekStart + i) % 7)
}

/** Whether `dayMs` is a day the habit asks for. "N per week" habits may be done on any day. */
export function isDue(habit: Pick<HabitBody, 'frequency'>, dayMs: number): boolean {
  const f = habit.frequency
  return f.kind === 'weekdays' ? f.days.includes(new Date(dayMs).getDay()) : true
}

const weekCount = (checked: ReadonlySet<string>, weekFirst: number): number => {
  let n = 0
  for (let i = 0; i < 7; i++) if (checked.has(dayKey(addDays(weekFirst, i)))) n++
  return n
}

// ---- streaks ---------------------------------------------------------------

/**
 * Current and best streak. A day the habit is not due neither counts nor
 * breaks. Today not being done yet does not break the streak (there is still
 * time), but any earlier missed due day does. N-per-week habits count weeks in
 * which at least `times` check-ins landed.
 */
export function computeStreak(habit: Pick<HabitBody, 'frequency'>, checked: ReadonlySet<string>, todayMs: number, weekStart: WeekStart): Streak {
  const today = startOfDay(todayMs)
  const first = [...checked].map(parseDayKey).filter((t): t is number => t !== null && t <= today).sort((a, b) => a - b)[0]
  const f = habit.frequency
  const unit = f.kind === 'perWeek' ? 'week' : 'day'
  if (first === undefined) return { current: 0, best: 0, unit }
  let run = 0
  let best = 0
  if (f.kind === 'perWeek') {
    const thisWeek = startOfWeek(today, weekStart)
    for (let w = startOfWeek(first, weekStart); w <= thisWeek; w = addDays(w, 7)) {
      if (weekCount(checked, w) >= f.times) {
        run++
        best = Math.max(best, run)
      } else if (w !== thisWeek) run = 0 // this week may still get there
    }
  } else {
    for (let d = first; d <= today; d = addDays(d, 1)) {
      if (checked.has(dayKey(d))) {
        run++
        best = Math.max(best, run)
      } else if (d !== today && isDue(habit, d)) run = 0
    }
  }
  return { current: run, best, unit }
}

// ---- goal text ------------------------------------------------------------------

const GOAL = /^(\d{1,4}) (.+) per day$/

export function formatGoal(count: number, unit: string): string {
  return `${count} ${unit.trim() || 'time'} per day`
}

/** Read a goal back into the dialog's fields; anything unrecognised becomes a plain 1 time. */
export function parseGoal(goal: string): { count: number; unit: string } {
  const m = GOAL.exec(goal)
  return m ? { count: Number(m[1]), unit: m[2]! } : { count: 1, unit: 'time' }
}

export function frequencyLabel(f: Frequency): string {
  switch (f.kind) {
    case 'daily':
      return 'Daily'
    case 'perWeek':
      return `${f.times} time${f.times === 1 ? '' : 's'} per week`
    case 'weekdays':
      return [...f.days].sort((a, b) => a - b).map((d) => ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][d]).join(', ')
  }
}

// ---- stored docs -----------------------------------------------------------------

export function asHabitBody(body: unknown): HabitBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.name !== 'string') return null
  const f = o.frequency as { kind?: unknown; days?: unknown; times?: unknown } | null | undefined
  let frequency: Frequency = { kind: 'daily' }
  if (f?.kind === 'weekdays' && Array.isArray(f.days)) frequency = { kind: 'weekdays', days: f.days.filter((d): d is number => Number.isInteger(d) && d >= 0 && d <= 6) }
  else if (f?.kind === 'perWeek' && typeof f.times === 'number') frequency = { kind: 'perWeek', times: Math.min(7, Math.max(1, Math.round(f.times))) }
  return {
    name: o.name,
    color: typeof o.color === 'string' ? o.color : '#4772fa',
    goal: typeof o.goal === 'string' ? o.goal : '1 time per day',
    frequency,
    reminder: typeof o.reminder === 'string' ? o.reminder : null,
    createdMs: typeof o.createdMs === 'number' ? o.createdMs : 0,
    archived: o.archived === true,
  }
}

export function asCheckinBody(body: unknown): Omit<Checkin, 'id'> | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.habitId !== 'string' || typeof o.day !== 'string' || parseDayKey(o.day) === null) return null
  return { habitId: o.habitId, day: o.day, count: typeof o.count === 'number' ? o.count : 1 }
}

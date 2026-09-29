/**
 * Numbers for the Statistics dialog, computed from the tasks in memory. Pure:
 * `now` and the week start are passed in.
 */
import type { List, Task } from '@/api/types'
import { addDays, dayKey, startOfDay, startOfWeek, type WeekStart } from '@/lib/date'

export interface ListCount {
  listId: string
  name: string
  count: number
}

export interface Stats {
  completedToday: number
  completedThisWeek: number
  completedAllTime: number
  open: number
  overdue: number
  /** Consecutive days, ending today (or yesterday, if nothing is done yet today), with at least one task completed. */
  streak: number
  topLists: ListCount[]
}

const TOP = 5

/** Overdue like the task list shows it: a timed task once its time has passed, an all-day one once its day has. */
const isOverdue = (t: Task, now: number): boolean => t.dueMs !== null && (t.isAllDay ? startOfDay(t.dueMs) < startOfDay(now) : t.dueMs < now)

export function computeStats(tasks: Task[], lists: List[], now: number, weekStart: WeekStart): Stats {
  const live = tasks.filter((t) => t.deletedMs === null)
  const done = live.filter((t) => t.status === 'done')
  const today = startOfDay(now)
  const week = startOfWeek(now, weekStart)

  const doneDays = new Set(done.filter((t) => t.completedMs !== null).map((t) => dayKey(t.completedMs!)))
  let day = doneDays.has(dayKey(today)) ? today : addDays(today, -1) // an unfinished today does not break a streak
  let streak = 0
  while (doneDays.has(dayKey(day))) {
    streak += 1
    day = addDays(day, -1)
  }

  const names = new Map(lists.map((l) => [l.id, l.name]))
  const counts = new Map<string, number>()
  for (const t of done) counts.set(t.listId, (counts.get(t.listId) ?? 0) + 1)
  const topLists = [...counts.entries()]
    .map(([listId, count]) => ({ listId, name: names.get(listId) ?? 'Inbox', count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
    .slice(0, TOP)

  return {
    completedToday: done.filter((t) => t.completedMs !== null && t.completedMs >= today && t.completedMs < addDays(today, 1)).length,
    completedThisWeek: done.filter((t) => t.completedMs !== null && t.completedMs >= week && t.completedMs < addDays(week, 7)).length,
    completedAllTime: done.length,
    open: live.length - done.length,
    overdue: live.filter((t) => t.status === 'open' && isOverdue(t, now)).length,
    streak,
    topLists,
  }
}

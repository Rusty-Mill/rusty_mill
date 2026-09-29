/**
 * Kanban: what dropping a card on another column changes. The columns are the
 * view's groups, so moving a card means editing the field the groups are made of.
 */
import type { Priority, Task, TaskPatch } from '@/api/types'
import { addDays, atTime, startOfDay } from '@/lib/date'
import type { GroupBy } from './organize'

/** Columns are made from the view's grouping; "none" has no columns of its own, so use priority. */
export const kanbanGroupBy = (by: GroupBy): Exclude<GroupBy, 'none'> => (by === 'none' ? 'priority' : by)

const PRIORITY_OF: Record<string, Priority> = { p5: 5, p3: 3, p1: 1, p0: 0 }

/**
 * The patch that moves `task` from column `from` to column `to` of a board
 * grouped by `by`, or `null` if the drop means nothing (same column, or a
 * column that cannot be set: "Overdue").
 */
export function dropPatch(task: Task, by: Exclude<GroupBy, 'none'>, from: string, to: string, now: number): TaskPatch | null {
  if (from === to) return null
  switch (by) {
    case 'priority': {
      const p = PRIORITY_OF[to]
      return p === undefined ? null : { priority: p }
    }
    case 'list':
      return { listId: to }
    case 'tag': {
      const tags = task.tags.filter((t) => t !== from)
      if (to !== 'notag' && !tags.includes(to)) tags.push(to)
      return { tags }
    }
    case 'date': {
      const today = startOfDay(now)
      const day = { today: today, tomorrow: addDays(today, 1), next7: addDays(today, 3), later: addDays(today, 14) }[to as 'today']
      if (to === 'nodate') return { dueMs: null, startMs: null, isAllDay: false }
      if (day === undefined) return null // Overdue is a state, not a date to pick
      // Keep a time of day if the task had one; otherwise the move makes it all-day.
      if (task.dueMs !== null && !task.isAllDay) {
        const d = new Date(task.dueMs)
        return { dueMs: atTime(day, d.getHours(), d.getMinutes()) }
      }
      return { dueMs: day, isAllDay: true }
    }
  }
}

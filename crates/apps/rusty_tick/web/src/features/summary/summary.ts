/**
 * Building a summary: which tasks fall in a range, and how they are laid out.
 * Pure (no DOM, no clock) so every rule is testable; rendering is in `render.ts`.
 *
 * A finished task belongs to the range its `completedMs` is in, an open one to
 * the range its `dueMs` is in (an open task with no due date has no place on a
 * timeline, so it is left out).
 */
import type { List, Priority, Task } from '@/api/types'
import { dayKey, formatTime, monthName, startOfDay, weekdayName, type WeekStart } from '@/lib/date'
import { TEMPLATE_LABELS, type SummaryOptions } from './options'
import { computeRange, formatRange, inRange, type DateRange } from './range'

export interface SummaryItem {
  title: string
  done: boolean
  /** Short facts shown after the title, in order (list, due date, priority, tags, completion time). */
  meta: string[]
}

export interface SummaryGroup {
  heading: string | null
  items: SummaryItem[]
}

export interface SummarySection {
  heading: string | null
  groups: SummaryGroup[]
}

export interface SummaryDoc {
  title: string
  subtitle: string
  sections: SummarySection[]
  /** Tasks in the summary; 0 means "nothing to report". */
  total: number
}

export interface SummaryContext {
  tasks: Task[]
  lists: List[]
  now: number
  weekStart: WeekStart
  hour12: boolean
}

const PRIORITY_LABEL: Record<Priority, string> = { 0: 'No priority', 1: 'Low priority', 3: 'Medium priority', 5: 'High priority' }

/** The tasks that match the filters, split into finished-in-range and due-in-range, each oldest first. */
export function selectTasks(tasks: Task[], o: SummaryOptions, range: DateRange): { completed: Task[]; open: Task[] } {
  const passes = (t: Task): boolean =>
    t.deletedMs === null &&
    (o.listIds.length === 0 || o.listIds.includes(t.listId)) &&
    (o.priorities.length === 0 || o.priorities.includes(t.priority)) &&
    (o.tags.length === 0 || t.tags.some((g) => o.tags.includes(g)))
  const byTitle = (a: Task, b: Task): number => a.title.localeCompare(b.title)
  const completed =
    o.status === 'open' ? [] : tasks.filter((t) => passes(t) && t.status === 'done' && inRange(t.completedMs, range)).sort((a, b) => a.completedMs! - b.completedMs! || byTitle(a, b))
  const open = o.status === 'completed' ? [] : tasks.filter((t) => passes(t) && t.status === 'open' && inRange(t.dueMs, range)).sort((a, b) => a.dueMs! - b.dueMs! || byTitle(a, b))
  return { completed, open }
}

const dateText = (ms: number): string => {
  const d = new Date(ms)
  return `${monthName(d.getMonth())} ${d.getDate()}`
}

export function buildSummary(o: SummaryOptions, cx: SummaryContext): SummaryDoc {
  const title = TEMPLATE_LABELS[o.template]
  const range = computeRange(o.range, cx.now, cx.weekStart, o.custom)
  if (!range) return { title, subtitle: 'Choose a valid date range.', sections: [], total: 0 }

  const { completed, open } = selectTasks(cx.tasks, o, range)
  const listName = new Map(cx.lists.map((l) => [l.id, l.name]))
  const listOrder = new Map(cx.lists.map((l, i) => [l.id, [l.sortOrder, i] as const]))

  const item = (t: Task): SummaryItem => {
    const meta: string[] = []
    if (o.showList && o.groupBy !== 'list') meta.push(listName.get(t.listId) ?? 'Inbox')
    if (o.showDue && t.dueMs !== null) meta.push(`Due ${dateText(t.dueMs)}${t.isAllDay ? '' : ` ${formatTime(t.dueMs, cx.hour12)}`}`)
    if (o.showPriority && t.priority !== 0) meta.push(PRIORITY_LABEL[t.priority])
    if (o.showTags && t.tags.length > 0) meta.push(t.tags.map((g) => `#${g}`).join(' '))
    if (o.showCompletedTime && t.status === 'done' && t.completedMs !== null) meta.push(`Done ${dateText(t.completedMs)} ${formatTime(t.completedMs, cx.hour12)}`)
    return { title: t.title, done: t.status === 'done', meta }
  }

  /** Split `tasks` (already in time order) into groups: by list, by day for the weekly report, or not at all. */
  const groups = (tasks: Task[], when: (t: Task) => number | null): SummaryGroup[] => {
    if (tasks.length === 0) return []
    if (o.groupBy === 'list') {
      const by = new Map<string, Task[]>()
      for (const t of tasks) by.set(t.listId, [...(by.get(t.listId) ?? []), t])
      const order = (id: string): [number, number] => {
        const k = listOrder.get(id)
        return k ? [k[0], k[1]] : [Number.MAX_SAFE_INTEGER, 0]
      }
      return [...by.entries()]
        .sort(([a], [b]) => order(a)[0] - order(b)[0] || order(a)[1] - order(b)[1])
        .map(([id, ts]) => ({ heading: listName.get(id) ?? 'Inbox', items: ts.map(item) }))
    }
    if (o.template === 'weekly') {
      const days = new Map<string, Task[]>()
      for (const t of tasks) {
        const k = dayKey(when(t) ?? 0)
        days.set(k, [...(days.get(k) ?? []), t])
      }
      return [...days.values()].map((ts) => {
        const ms = startOfDay(when(ts[0]!) ?? 0)
        return { heading: `${weekdayName(new Date(ms).getDay())}, ${dateText(ms)}`, items: ts.map(item) }
      })
    }
    return [{ heading: null, items: tasks.map(item) }]
  }

  const sections: SummarySection[] = []
  if (o.template === 'simple') {
    const all = [...completed, ...open]
    if (all.length > 0) sections.push({ heading: null, groups: groups(all, (t) => t.completedMs ?? t.dueMs) })
  } else {
    if (completed.length > 0) sections.push({ heading: `Completed (${completed.length})`, groups: groups(completed, (t) => t.completedMs) })
    if (open.length > 0) sections.push({ heading: `Not completed (${open.length})`, groups: groups(open, (t) => t.dueMs) })
  }
  return { title, subtitle: formatRange(range, cx.now), sections, total: completed.length + open.length }
}

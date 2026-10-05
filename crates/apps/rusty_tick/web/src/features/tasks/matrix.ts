/**
 * Eisenhower matrix: which of four quadrants a task falls in, and what dropping
 * it in another quadrant changes. Important = medium or high priority; urgent =
 * due today or overdue. Pure, like `kanban.ts`.
 */
import type { Task, TaskPatch } from '@/api/types'
import { dropPatch } from './kanban'
import { dateBucket } from './organize'

export type Quadrant = 'do' | 'plan' | 'delegate' | 'drop'

export const QUADRANTS: { key: Quadrant; label: string; hint: string; important: boolean; urgent: boolean }[] = [
  { key: 'do', label: 'Do First', hint: 'Important · Urgent', important: true, urgent: true },
  { key: 'plan', label: 'Schedule', hint: 'Important · Not urgent', important: true, urgent: false },
  { key: 'delegate', label: 'Delegate', hint: 'Not important · Urgent', important: false, urgent: true },
  { key: 'drop', label: 'Eliminate', hint: 'Not important · Not urgent', important: false, urgent: false },
]

const isImportant = (t: Task): boolean => t.priority >= 3
const isUrgent = (t: Task, now: number): boolean => ['overdue', 'today'].includes(dateBucket(t, now))

export function quadrantOf(t: Task, now: number): Quadrant {
  const imp = isImportant(t)
  const urg = isUrgent(t, now)
  return imp ? (urg ? 'do' : 'plan') : urg ? 'delegate' : 'drop'
}

/** `tasks` split into the four quadrants, in the order of {@link QUADRANTS}; input order is kept. */
export function splitQuadrants(tasks: Task[], now: number): Record<Quadrant, Task[]> {
  const out: Record<Quadrant, Task[]> = { do: [], plan: [], delegate: [], drop: [] }
  for (const t of tasks) out[quadrantOf(t, now)].push(t)
  return out
}

/**
 * The patch for dropping `task` on quadrant `to`, or `null` if nothing changes.
 * Only the axes that differ are touched: importance sets high priority (or none),
 * urgency sets the due date to today (or moves an urgent task into the next 7 days).
 */
export function matrixDropPatch(task: Task, to: Quadrant, now: number): TaskPatch | null {
  const want = QUADRANTS.find((q) => q.key === to)!
  const patch: TaskPatch = {}
  if (want.important !== isImportant(task)) patch.priority = want.important ? 5 : 0
  if (want.urgent !== isUrgent(task, now)) Object.assign(patch, dropPatch(task, 'date', '', want.urgent ? 'today' : 'next7', now))
  return Object.keys(patch).length === 0 ? null : patch
}

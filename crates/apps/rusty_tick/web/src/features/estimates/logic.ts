/** Pomodoro estimates per task, against the pomos actually recorded. Pure. */
import type { Task } from '@/api/types'
import type { FocusRecordBody } from '../focus/logic'

import { derivedId } from '@/lib/id'

export interface EstimateBody {
  taskId: string
  pomos: number
}

export const MAX_ESTIMATE = 99

/** The id of a task's estimate doc: its own per task, and different from the task's other docs. */
export const estimateDocId = (taskId: string): string => derivedId(`estimate|${taskId}`)

/** The estimates with one per task (if a task has two docs, the later one wins). */
export const latestByTask = (items: readonly EstimateBody[]): EstimateBody[] => [...new Map(items.map((e) => [e.taskId, e])).values()]

export function asEstimateBody(body: unknown): EstimateBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.taskId !== 'string' || typeof o.pomos !== 'number' || !Number.isFinite(o.pomos)) return null
  const pomos = Math.min(MAX_ESTIMATE, Math.round(o.pomos))
  return pomos > 0 ? { taskId: o.taskId, pomos } : null
}

/** Finished pomos recorded against `taskId` (stopwatch time does not count). */
export const actualPomos = (records: readonly FocusRecordBody[], taskId: string): number =>
  records.filter((r) => r.kind === 'pomo' && r.taskId === taskId).length

export interface EstimateRow {
  taskId: string
  title: string
  estimated: number
  actual: number
}

/** One row per estimate whose task still exists, those over their estimate first, then by title. */
export function estimateRows(estimates: readonly EstimateBody[], records: readonly FocusRecordBody[], tasks: Record<string, Task>): EstimateRow[] {
  const rows = latestByTask(estimates).flatMap((e): EstimateRow[] => {
    const t = tasks[e.taskId]
    return t && t.deletedMs === null ? [{ taskId: e.taskId, title: t.title, estimated: e.pomos, actual: actualPomos(records, e.taskId) }] : []
  })
  const over = (r: EstimateRow): number => Number(r.actual > r.estimated)
  return rows.sort((a, b) => over(b) - over(a) || a.title.localeCompare(b.title))
}

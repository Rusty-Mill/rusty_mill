/** Pomodoro estimates as `estimate` client documents, keyed by the task's id (one per task). */
import { createDocStore } from '@/lib/docStore'
import { asEstimateBody, MAX_ESTIMATE } from './logic'

const { useDocs, reset } = createDocStore('estimate', asEstimateBody, 'estimate')
export const useEstimates = useDocs
export const resetEstimatesStore = reset

/** Set a task's estimate; zero or less clears it. */
export function setEstimate(taskId: string, pomos: number): void {
  const n = Math.min(MAX_ESTIMATE, Math.round(pomos))
  if (n > 0) useDocs.getState().put(taskId, { taskId, pomos: n })
  else useDocs.getState().remove(taskId)
}

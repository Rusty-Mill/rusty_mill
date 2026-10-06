/** Pomodoro estimates as `estimate` client documents, one per task. */
import { createDocStore } from '@/lib/docStore'
import { asEstimateBody, estimateDocId, MAX_ESTIMATE } from './logic'

const { useDocs, reset } = createDocStore('estimate', asEstimateBody, 'estimate')
export const useEstimates = useDocs
export const resetEstimatesStore = reset

/**
 * Set a task's estimate; zero or less clears it. A doc an earlier version
 * saved under the bare task id (which clashed with the task's other docs) is removed.
 */
export function setEstimate(taskId: string, pomos: number): void {
  const docs = useDocs.getState()
  const id = estimateDocId(taskId)
  if (docs.items.some((i) => i.id === taskId)) docs.remove(taskId)
  const n = Math.min(MAX_ESTIMATE, Math.round(pomos))
  if (n > 0) useDocs.getState().put(id, { taskId, pomos: n })
  else useDocs.getState().remove(id)
}

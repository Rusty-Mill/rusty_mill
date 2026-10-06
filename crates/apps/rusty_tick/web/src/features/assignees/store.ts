/** Task assignees as `assignee` client documents, one per task. */
import { createDocStore } from '@/lib/docStore'
import { asAssigneeBody, assigneeDocId, cleanName } from './logic'

const { useDocs, reset } = createDocStore('assignee', asAssigneeBody, 'assignee')
export const useAssignees = useDocs
export const resetAssigneesStore = reset

/**
 * Assign a task to `name`; a blank name unassigns it. A doc an earlier version
 * saved under the bare task id (which clashed with the task's other docs) is removed.
 */
export function setAssignee(taskId: string, name: string): void {
  const docs = useDocs.getState()
  const id = assigneeDocId(taskId)
  if (docs.items.some((i) => i.id === taskId)) docs.remove(taskId)
  const clean = cleanName(name)
  if (clean) useDocs.getState().put(id, { taskId, name: clean })
  else useDocs.getState().remove(id)
}

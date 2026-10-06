/** Task assignees as `assignee` client documents, one per task (the doc id is the task id). */
import { createDocStore } from '@/lib/docStore'
import { asAssigneeBody, cleanName } from './logic'

const { useDocs, reset } = createDocStore('assignee', asAssigneeBody, 'assignee')
export const useAssignees = useDocs
export const resetAssigneesStore = reset

/** Assign a task to `name`; a blank name unassigns it. */
export function setAssignee(taskId: string, name: string): void {
  const clean = cleanName(name)
  if (clean) useDocs.getState().put(taskId, { taskId, name: clean })
  else useDocs.getState().remove(taskId)
}

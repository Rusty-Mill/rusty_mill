/** Choosing the task a duplicate is merged into. Pure. */
import type { Task } from '@/api/types'

const norm = (text: string): string => text.trim().toLowerCase()

/**
 * Tasks `duplicate` may be merged into: live tasks other than itself. With no
 * `query`, those with its title (the likely duplicates); else those whose
 * title contains `query`, its title first. At most `limit`.
 */
export function mergeTargets(tasks: readonly Task[], duplicate: Task, query: string, limit = 50): Task[] {
  const title = norm(duplicate.title)
  const q = norm(query)
  const matches = (t: Task): boolean => (q === '' ? norm(t.title) === title : norm(t.title).includes(q))
  return tasks
    .filter((t) => t.id !== duplicate.id && t.deletedMs === null && matches(t))
    .sort((a, b) => Number(norm(b.title) === title) - Number(norm(a.title) === title))
    .slice(0, limit)
}

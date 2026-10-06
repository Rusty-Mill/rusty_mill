/**
 * Who a task is assigned to: a free-text name per task (this app has no
 * accounts to pick from within one data store), kept as `assignee` client
 * documents keyed by the task's id so the task record itself is unchanged.
 */
export interface AssigneeBody {
  taskId: string
  name: string
}

export const MAX_NAME = 60

/** The value a filter uses for "no assignee". */
export const UNASSIGNED = '__unassigned__'

export const cleanName = (raw: string): string => raw.trim().replace(/\s+/g, ' ').slice(0, MAX_NAME)

export function asAssigneeBody(body: unknown): AssigneeBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.taskId !== 'string' || typeof o.name !== 'string') return null
  const name = cleanName(o.name)
  return name ? { taskId: o.taskId, name } : null
}

/** Assignee name by task id. */
export const assigneeMap = (items: readonly AssigneeBody[]): Record<string, string> => Object.fromEntries(items.map((a) => [a.taskId, a.name]))

/** Every name in use, once each (case-insensitively, keeping the first spelling), sorted. */
export function knownNames(items: readonly AssigneeBody[]): string[] {
  const seen = new Map<string, string>()
  for (const a of items) if (!seen.has(a.name.toLowerCase())) seen.set(a.name.toLowerCase(), a.name)
  return [...seen.values()].sort((a, b) => a.localeCompare(b))
}

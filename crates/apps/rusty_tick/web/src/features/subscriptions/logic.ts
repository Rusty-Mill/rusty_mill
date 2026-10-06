/**
 * Calendar subscriptions: a feed URL, the list its events live in, and which
 * task came from which event (`items`, by the feed's UID), so a refresh
 * updates tasks in place instead of duplicating them. Pure.
 */
import type { Task, TaskPatch } from '@/api/types'
import type { IcsTask } from '@/lib/ics'

export interface SubscriptionBody {
  name: string
  url: string
  listId: string
  /** Feed UID to task id. */
  items: Record<string, string>
  /** When the last successful sync finished; 0 before the first. */
  syncedMs: number
}

/** A feed is read at most this many events deep, so `items` stays inside a doc's size limit. */
export const MAX_ITEMS = 500

export function asSubscriptionBody(body: unknown): SubscriptionBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.name !== 'string' || typeof o.url !== 'string' || typeof o.listId !== 'string' || !o.name.trim() || !o.url.trim()) return null
  const items: Record<string, string> = {}
  if (typeof o.items === 'object' && o.items !== null) {
    for (const [uid, id] of Object.entries(o.items)) if (typeof id === 'string') items[uid] = id
  }
  return { name: o.name, url: o.url, listId: o.listId, items, syncedMs: typeof o.syncedMs === 'number' ? o.syncedMs : 0 }
}

/** The task fields a feed owns; a sync overwrites these and nothing else. */
export const sourceFields = (t: IcsTask): Pick<Task, 'title' | 'notes' | 'startMs' | 'dueMs' | 'isAllDay' | 'timeZone' | 'repeatFlag'> => ({
  title: t.title,
  notes: t.notes,
  startMs: t.startMs,
  dueMs: t.dueMs,
  isAllDay: t.isAllDay,
  timeZone: t.timeZone,
  repeatFlag: t.repeatFlag,
})

export interface SyncPlan {
  create: IcsTask[]
  update: { taskId: string; patch: TaskPatch }[]
  /** Tasks whose event left the feed. */
  trash: string[]
  /** The mapping to store afterwards. */
  items: Record<string, string>
}

/**
 * What to do so the tasks match `feed`:
 * - a new UID creates a task;
 * - a known UID updates the fields the feed owns, if any changed (a task the
 *   user deleted stays deleted: it is not brought back);
 * - a UID gone from the feed trashes its task.
 */
export function planSync(feed: readonly IcsTask[], items: Record<string, string>, tasks: Record<string, Task>): SyncPlan {
  const plan: SyncPlan = { create: [], update: [], trash: [], items: {} }
  const seen = new Set<string>()
  for (const event of feed.slice(0, MAX_ITEMS)) {
    if (seen.has(event.uid)) continue
    seen.add(event.uid)
    const taskId = items[event.uid]
    if (taskId === undefined) {
      plan.create.push(event)
      continue
    }
    plan.items[event.uid] = taskId
    const task = tasks[taskId]
    if (!task || task.deletedMs !== null) continue
    const want = sourceFields(event)
    const patch: Record<string, unknown> = {}
    for (const key of Object.keys(want) as (keyof typeof want)[]) if (task[key] !== want[key]) patch[key] = want[key]
    if (Object.keys(patch).length > 0) plan.update.push({ taskId, patch: patch as TaskPatch })
  }
  for (const [uid, taskId] of Object.entries(items)) {
    if (seen.has(uid)) continue
    const task = tasks[taskId]
    if (task && task.deletedMs === null) plan.trash.push(taskId)
  }
  return plan
}

/** Whether a subscription is due an automatic refresh. */
export const isStale = (s: SubscriptionBody, now: number, maxAgeMs: number): boolean => now - s.syncedMs > maxAgeMs

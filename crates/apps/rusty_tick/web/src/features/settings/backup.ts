import type { ApiClient } from '@/api/client'
import type { DocKind, Snapshot, Task } from '@/api/types'

const DOC_KINDS: DocKind[] = ['habit', 'habit_checkin', 'focus', 'prefs', 'summary_template', 'comment']

export interface Restored {
  lists: number
  tags: number
  tasks: number
}

const isSnapshot = (v: unknown): v is Snapshot => {
  const s = v as Partial<Snapshot> | null
  return !!s && typeof s.inboxId === 'string' && Array.isArray(s.lists) && Array.isArray(s.tasks) && Array.isArray(s.tags)
}

/** Parents before children, so a subtask never names a parent that does not exist yet. */
const parentsFirst = (tasks: Task[]): Task[] => [...tasks.filter((t) => !t.parentId), ...tasks.filter((t) => t.parentId)]

/**
 * Add what a backup file holds and this account lacks. Ids are kept, so
 * importing the same file twice changes nothing, and nothing already here is
 * overwritten. Tasks in the file's Inbox land in this account's Inbox.
 */
export async function restoreBackup(api: ApiClient, text: string): Promise<Restored> {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    throw new Error('That file is not a Tick Local backup')
  }
  if (!isSnapshot(parsed)) throw new Error('That file is not a Tick Local backup')
  const have = await api.snapshot()
  const listIds = new Set(have.lists.map((l) => l.id))
  const taskIds = new Set(have.tasks.map((t) => t.id))
  const tagNames = new Set(have.tags.map((t) => t.name))
  const out: Restored = { lists: 0, tags: 0, tasks: 0 }

  for (const l of parsed.lists) {
    if (l.id === parsed.inboxId || listIds.has(l.id)) continue
    await api.createList({ id: l.id, name: l.name, color: l.color })
    listIds.add(l.id)
    out.lists++
  }
  for (const t of parsed.tags) {
    if (tagNames.has(t.name)) continue
    await api.createTag(t.label, t.color)
    out.tags++
  }
  for (const t of parentsFirst(parsed.tasks.filter((t) => t.deletedMs === null))) {
    if (taskIds.has(t.id)) continue
    const listId = t.listId === parsed.inboxId ? have.inboxId : t.listId
    await api.createTask({ id: t.id, listId, parentId: t.parentId, title: t.title, notes: t.notes, kind: t.kind, priority: t.priority, startMs: t.startMs, dueMs: t.dueMs, isAllDay: t.isAllDay, timeZone: t.timeZone, reminders: t.reminders, repeatFlag: t.repeatFlag, items: t.items, tags: t.tags })
    if (t.status !== 'open') await api.updateTask(t.id, { status: t.status })
    out.tasks++
  }
  return out
}

/** Delete every task, list, tag and document. The account and its token stay; the Inbox is emptied, not removed. */
export async function wipeData(api: ApiClient): Promise<void> {
  const snap = await api.snapshot()
  for (const t of snap.tasks.filter((t) => !t.parentId)) await api.purgeTask(t.id)
  for (const l of snap.lists.filter((l) => l.id !== snap.inboxId)) await api.deleteList(l.id)
  for (const t of snap.tags) await api.deleteTag(t.name)
  await api.emptyTrash()
  for (const kind of DOC_KINDS) for (const d of await api.listDocs(kind)) await api.deleteDoc(kind, d.id)
}

import type { ApiClient } from '@/api/client'
import type { Doc, DocKind, Snapshot, Task } from '@/api/types'

const DOC_KINDS: DocKind[] = ['habit', 'habit_checkin', 'focus', 'prefs', 'summary_template', 'comment']

export interface Restored {
  lists: number
  tags: number
  tasks: number
  docs: number
}

/** What a backup file holds: the snapshot, plus the client documents (habits, check-ins, comments, ...) the snapshot does not carry. */
export interface Backup extends Snapshot {
  docs?: Partial<Record<DocKind, Doc[]>>
}

/** Everything the account holds that can be restored elsewhere. */
export async function makeBackup(api: ApiClient): Promise<Backup> {
  const [snap, ...lists] = await Promise.all([api.snapshot(), ...DOC_KINDS.map((k) => api.listDocs(k))])
  return { ...snap, docs: Object.fromEntries(DOC_KINDS.map((k, i) => [k, lists[i]])) }
}

const isDoc = (v: unknown): v is Doc => {
  const d = v as Partial<Doc> | null
  return !!d && typeof d.id === 'string' && d.body !== undefined
}

const isSnapshot = (v: unknown): v is Backup => {
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
  const out: Restored = { lists: 0, tags: 0, tasks: 0, docs: 0 }

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
    taskIds.add(t.id)
    out.tasks++
  }
  out.docs = await restoreDocs(api, parsed.docs, taskIds)
  return out
}

/** Put back documents the account lacks. A comment whose task is not here (it was in the trash, say) would only be swept, so it is left out. */
async function restoreDocs(api: ApiClient, docs: Backup['docs'], taskIds: ReadonlySet<string>): Promise<number> {
  let n = 0
  for (const kind of DOC_KINDS) {
    const incoming = docs?.[kind]
    if (!Array.isArray(incoming)) continue
    const have = new Set((await api.listDocs(kind)).map((d) => d.id))
    for (const d of incoming.filter(isDoc)) {
      const owner = (d.body as { taskId?: string } | null)?.taskId
      if (have.has(d.id) || (kind === 'comment' && !taskIds.has(owner ?? ''))) continue
      await api.putDoc(kind, d.id, d.body)
      n++
    }
  }
  return n
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

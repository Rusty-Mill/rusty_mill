/**
 * Write an `ImportResult` through the API. Task and list ids are derived from
 * the source and its keys, so importing the same file again adds nothing and
 * overwrites nothing. Lists are matched by name (case-insensitively) before one
 * is created; `''` and unknown parents fall back sensibly: the Inbox, and a
 * top-level task. Subtasks nest one level here, so deeper ones hang off their
 * top-level ancestor and live in its list.
 */
import type { ApiClient } from '@/api/client'
import type { NewTask } from '@/api/types'
import { derivedId } from '@/lib/id'
import type { ImportResult, ImportedTask } from './model'

/** The server's limits (`service.rs`), so one long field does not fail a whole import. */
const MAX_TITLE = 500
const MAX_TAGS = 20
const MAX_TAG_CHARS = 64
const MAX_ITEMS = 200

export interface Applied {
  tasks: number
  lists: number
  /** Already imported (same id), so left alone. */
  existing: number
}

/** The top-level ancestor's key, or `null` when `t` is top-level or its parent is not in the file. */
function rootOf(t: ImportedTask, byKey: ReadonlyMap<string, ImportedTask>): string | null {
  let parent = t.parentKey !== null ? byKey.get(t.parentKey) : undefined
  for (let hops = 0; parent?.parentKey && byKey.has(parent.parentKey) && hops < byKey.size; hops++) parent = byKey.get(parent.parentKey)
  return parent && parent.key !== t.key ? parent.key : null
}

function toNewTask(t: ImportedTask, id: string, listId: string, parentId: string | null): NewTask {
  return {
    id,
    listId,
    parentId,
    title: t.title.slice(0, MAX_TITLE),
    notes: t.notes,
    kind: t.items.length > 0 ? 'checklist' : 'text',
    priority: t.priority,
    startMs: t.startMs,
    dueMs: t.dueMs,
    isAllDay: t.isAllDay,
    timeZone: t.timeZone,
    reminders: t.reminders,
    repeatFlag: t.repeatFlag,
    items: t.items.slice(0, MAX_ITEMS).map((item, i) => ({ id: derivedId(`${id}|item|${i}`), title: item.title.slice(0, MAX_TITLE), done: item.done, sortOrder: i })),
    tags: t.tags.filter((tag) => tag.length <= MAX_TAG_CHARS).slice(0, MAX_TAGS),
  }
}

export async function applyImport(api: ApiClient, result: ImportResult): Promise<Applied> {
  const have = await api.snapshot()
  const lists = new Map(have.lists.map((l) => [l.name.trim().toLowerCase(), l.id]))
  const listIds = new Set(have.lists.map((l) => l.id))
  const taskIds = new Set(have.tasks.map((t) => t.id))
  const byKey = new Map(result.tasks.map((t) => [t.key, t]))
  const idOf = (key: string): string => derivedId(`import|${result.source}|${key}`)
  const out: Applied = { tasks: 0, lists: 0, existing: 0 }

  const listFor = async (name: string): Promise<string> => {
    const wanted = name.trim()
    if (!wanted) return have.inboxId
    const known = lists.get(wanted.toLowerCase())
    if (known) return known
    const id = derivedId(`import|list|${wanted.toLowerCase()}`)
    if (!listIds.has(id)) {
      await api.createList({ id, name: wanted }) // else imported before and since renamed: reuse it
      out.lists++
    }
    lists.set(wanted.toLowerCase(), id)
    return id
  }

  const placed = new Map<string, string>() // task key -> list id, so subtasks follow their parent
  const roots = result.tasks.map((t) => ({ t, root: rootOf(t, byKey) }))
  for (const { t, root: wanted } of [...roots.filter((r) => !r.root), ...roots.filter((r) => r.root)]) {
    const id = idOf(t.key)
    const root = wanted !== null && placed.has(wanted) ? wanted : null // a parent caught in a cycle was never placed
    const listId = (root !== null && placed.get(root)) || (await listFor(t.list))
    placed.set(t.key, listId)
    if (taskIds.has(id)) {
      out.existing++
      continue
    }
    await api.createTask(toNewTask(t, id, listId, root ? idOf(root) : null))
    if (t.status !== 'open') await api.updateTask(id, { status: t.status })
    taskIds.add(id)
    out.tasks++
  }
  return out
}

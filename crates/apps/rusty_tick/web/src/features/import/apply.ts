/**
 * Write an `ImportResult` through the API. Task and list ids are derived from
 * the source and its keys, so importing the same file again adds nothing and
 * overwrites nothing. Lists are matched by name (case-insensitively) before one
 * is created; `''` and unknown parents fall back sensibly: the Inbox, and a
 * top-level task. Subtasks nest one level here, so deeper ones hang off their
 * top-level ancestor and live in its list.
 *
 * A task is created open, then closed and given its repeat exceptions by a
 * second write, since `NewTask` carries neither status nor `exDates`. If that
 * second write was lost, the next import finishes it: the task is still at its
 * creation version (`etag` 1), so nobody has edited it, and the write is
 * conditional on that etag. A task edited since is left alone.
 *
 * Rows whose key format changed carry their earlier keys (`legacy`). An
 * earlier importer wrote rows in the order below (top-level tasks first) and
 * skipped a row whose key was already taken, so the first row in that order
 * that may claim a legacy key owns the task it names: any row, or with a
 * `dueMs`, only one whose due date the task still has. A task edited since
 * (its due date, title or repeat) goes to the one row left that it fits (`edited`).
 * A row whose own id is not here takes over the task it owns, so files
 * imported before still skip or resume rather than duplicate.
 */
import type { ApiClient } from '@/api/client'
import type { NewTask, Task, TaskPatch } from '@/api/types'
import { derivedId } from '@/lib/id'
import { storedExDates } from '@/lib/ics'
import type { ImportResult, ImportedTask, LegacyKey } from './model'

/** The server's limits (`service.rs`), so one long field does not fail a whole import. */
const MAX_TITLE = 500
const MAX_TAGS = 20
const MAX_TAG_CHARS = 64
const MAX_ITEMS = 200
/** A task's etag when it has not been written since it was created. */
const CREATED_ETAG = '1'

export interface Applied {
  tasks: number
  lists: number
  /** Already imported (same id), so left alone. */
  existing: number
  /** Imported before, but its status / repeat-exception write was lost; finished now. */
  resumed: number
}

/** What `NewTask` cannot carry, written right after the create; `null` when there is nothing to write. */
function secondWrite(t: ImportedTask): TaskPatch | null {
  const patch: TaskPatch = {}
  if (t.status !== 'open') patch.status = t.status
  if (t.exDates.length > 0) patch.exDates = storedExDates(t.exDates)
  return Object.keys(patch).length > 0 ? patch : null
}

/** True when `found` is untouched since its creation, so a second write it still lacks can be finished safely. */
const pending = (found: Task, patch: TaskPatch | null): patch is TaskPatch => patch !== null && found.etag === CREATED_ETAG

/** The top-level ancestor's key, or `null` when `t` is top-level or its parent is not in the file. */
function rootOf(t: ImportedTask, byKey: ReadonlyMap<string, ImportedTask>): string | null {
  let parent = t.parentKey !== null ? byKey.get(t.parentKey) : undefined
  for (let hops = 0; parent?.parentKey && byKey.has(parent.parentKey) && hops < byKey.size; hops++) parent = byKey.get(parent.parentKey)
  return parent && parent.key !== t.key ? parent.key : null
}

/**
 * How sure a pairing of an edited task and a row is, surest first: the same
 * title (as written) and both repeating or neither; then the repeat alone;
 * then only the UID, which every candidate shares with its row already.
 */
const sameRepeat = (found: Task, t: ImportedTask): boolean => (found.repeatFlag !== '') === (t.repeatFlag !== '')
const sameTitle = (found: Task, t: ImportedTask): boolean => sameRepeat(found, t) && found.title === t.title.slice(0, MAX_TITLE)
const sameUid = (): boolean => true

/**
 * Earlier tasks named by a due-dated legacy key that nobody claimed by due
 * date, paired with rows still without a task that they `fit`. Only a task
 * edited since its import qualifies: one still at its creation version has its
 * imported due date, so its row is gone from the file, not edited.
 *
 * Rows are paired a group at a time: the rows whose candidates are the same
 * tasks, when there are as many tasks as rows and no other row wants them. One
 * row and one task is a sure pair. Several are look-alikes, and which takes
 * which does not matter: each is edited, so none is written, and every row is
 * accounted for. When the counts differ, which row is new is a guess, so the
 * group is imported anew instead.
 */
function edited(
  rows: readonly ImportedTask[],
  existing: ReadonlyMap<string, Task>,
  owner: ReadonlyMap<string, string>,
  idOf: (key: string) => string,
  resolve: (t: ImportedTask) => string,
  fits: (found: Task, t: ImportedTask) => boolean,
): [id: string, key: string][] {
  const candidates = rows
    .filter((t) => !existing.has(resolve(t)))
    .map((t) => {
      const ids = t.legacy.filter((c) => c.dueMs !== undefined).map((c) => idOf(c.key))
      const found = ids.filter((id) => {
        const task = existing.get(id)
        return task !== undefined && task.etag !== CREATED_ETAG && !owner.has(id) && fits(task, t)
      })
      return { key: t.key, ids: [...new Set(found)] }
    })
  const wanted = new Map<string, number>()
  for (const c of candidates) for (const id of c.ids) wanted.set(id, (wanted.get(id) ?? 0) + 1)
  const groups = new Map<string, { ids: string[]; keys: string[] }>() // same candidates (in legacy-key order) -> their rows
  for (const c of candidates.filter((c) => c.ids.length > 0)) {
    const group = groups.get(c.ids.join('\n')) ?? { ids: c.ids, keys: [] }
    group.keys.push(c.key)
    groups.set(c.ids.join('\n'), group)
  }
  const whole = (g: { ids: string[]; keys: string[] }): boolean => g.ids.length === g.keys.length && g.ids.every((id) => wanted.get(id) === g.keys.length)
  return [...groups.values()].filter(whole).flatMap((g) => g.ids.map((id, i): [string, string] => [id, g.keys[i]!]))
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
  const existing = new Map(have.tasks.map((t) => [t.id, t]))
  const byKey = new Map(result.tasks.map((t) => [t.key, t]))
  const memo = new Map<string, string>()
  const idOf = (key: string): string => {
    const known = memo.get(key)
    if (known !== undefined) return known
    const id = derivedId(`import|${result.source}|${key}`)
    memo.set(key, id)
    return id
  }
  const roots = result.tasks.map((t) => ({ t, root: rootOf(t, byKey) }))
  const order = [...roots.filter((r) => !r.root), ...roots.filter((r) => r.root)] // #821's write order too
  const mayClaim = (c: LegacyKey): boolean => c.dueMs === undefined || existing.get(idOf(c.key))?.dueMs === c.dueMs
  const owner = new Map<string, string>() // legacy id -> key of the row that owns the task it names
  for (const { t } of order) for (const c of t.legacy) if (mayClaim(c) && !owner.has(idOf(c.key))) owner.set(idOf(c.key), t.key)
  /** The id a row writes to: its own, else the earlier task it owns, if that is here. */
  const resolve = (t: ImportedTask): string => {
    const own = idOf(t.key)
    if (existing.has(own)) return own
    const legacy = t.legacy.map((c) => idOf(c.key)).find((id) => owner.get(id) === t.key && existing.has(id))
    return legacy ?? own
  }
  for (const fits of [sameTitle, sameRepeat, sameUid]) {
    for (const [id, key] of edited(order.map((r) => r.t), existing, owner, idOf, resolve, fits)) owner.set(id, key)
  }
  const ids = new Map<string, string>() // task key -> resolved id, so subtasks point at their parent's real id
  const out: Applied = { tasks: 0, lists: 0, existing: 0, resumed: 0 }

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
  for (const { t, root: wanted } of order) {
    const id = resolve(t)
    ids.set(t.key, id)
    const root = wanted !== null && placed.has(wanted) ? wanted : null // a parent caught in a cycle was never placed
    const listId = (root !== null && placed.get(root)) || (await listFor(t.list))
    placed.set(t.key, listId)
    const found = existing.get(id)
    const patch = secondWrite(t)
    if (found && pending(found, patch)) {
      await api.updateTask(id, patch, CREATED_ETAG)
      out.resumed++
      continue
    }
    if (found) {
      out.existing++
      continue
    }
    const created = await api.createTask(toNewTask(t, id, listId, root !== null ? (ids.get(root) ?? null) : null))
    existing.set(id, created)
    if (patch) await api.updateTask(id, patch, created.etag)
    out.tasks++
  }
  return out
}

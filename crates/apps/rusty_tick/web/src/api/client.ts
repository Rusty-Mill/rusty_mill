import type {
  Doc,
  DocKind,
  List,
  ListPatch,
  NewList,
  NewTask,
  Snapshot,
  Tag,
  TagPatch,
  Task,
  TaskPatch,
} from './types'

/**
 * What the UI needs from a backend. `HttpAdapter` talks to `rusty_tick`;
 * `MemoryAdapter` implements the same rules in the browser so the UI runs
 * standalone and in tests. `apiContract.ts` holds the shared behaviour tests.
 *
 * `etag` arguments turn into `If-Match`: when given and stale, the call
 * rejects with `StaleError` carrying the current record.
 */
export interface ApiClient {
  /** Everything needed to boot: lists, tasks (trash and completed included), tags. */
  snapshot(): Promise<Snapshot>

  createList(input: NewList): Promise<List>
  updateList(id: string, patch: ListPatch, etag?: string): Promise<List>
  /** Its tasks go to the trash. The Inbox cannot be deleted. */
  deleteList(id: string): Promise<void>

  createTask(input: NewTask): Promise<Task>
  updateTask(id: string, patch: TaskPatch, etag?: string): Promise<Task>
  /** Move a task and its subtasks to the trash. */
  trashTask(id: string, etag?: string): Promise<void>
  restoreTask(id: string): Promise<Task>
  purgeTask(id: string): Promise<void>
  emptyTrash(): Promise<number>

  createTag(label: string, color?: string | null): Promise<Tag>
  updateTag(name: string, patch: TagPatch, etag?: string): Promise<Tag>
  /** Renames on every task that carries it. */
  renameTag(name: string, label: string): Promise<Tag>
  deleteTag(name: string): Promise<void>

  listDocs<T = unknown>(kind: DocKind): Promise<Doc<T>[]>
  putDoc<T = unknown>(kind: DocKind, id: string, body: T): Promise<Doc<T>>
  deleteDoc(kind: DocKind, id: string): Promise<void>

  /** The text of the calendar feed at `url`, fetched by the server (browsers cannot read most feeds). */
  fetchIcs(url: string): Promise<string>
}

/**
 * The wire model, exactly as `rusty_tick`'s API speaks it: camelCase, ids are
 * UUIDs, times are Unix milliseconds, and `etag` is the record's version.
 *
 * (The reference prompt sketched TickTick's own model — 24-hex ids, ISO dates,
 * `status: 0|2`. The backend here is `rusty_tick` on `rusty_multimodal_db`, so
 * the UI follows its contract instead; see README "Where this differs".)
 */

export type Status = 'open' | 'done'
export type Priority = 0 | 1 | 3 | 5
export type TaskKind = 'text' | 'checklist' | 'note'
export type ViewMode = 'list' | 'kanban' | 'timeline'

export interface ChecklistItem {
  id: string
  title: string
  done: boolean
  sortOrder: number
}

export interface Task {
  id: string
  listId: string
  parentId: string | null
  title: string
  notes: string
  kind: TaskKind
  status: Status
  priority: Priority
  startMs: number | null
  dueMs: number | null
  isAllDay: boolean
  timeZone: string
  reminders: string[]
  repeatFlag: string
  exDates: number[]
  items: ChecklistItem[]
  tags: string[]
  sortOrder: number
  createdMs: number
  updatedMs: number
  completedMs: number | null
  deletedMs: number | null
  etag: string
}

export interface List {
  id: string
  name: string
  color: string | null
  archived: boolean
  viewMode: ViewMode
  sortType: string
  sortOrder: number
  updatedMs: number
  etag: string
}

export interface Tag {
  /** Lowercased key; tasks refer to tags by this. */
  name: string
  /** The display form as typed. */
  label: string
  color: string | null
  parent: string | null
  sortOrder: number
  etag: string
}

export interface Snapshot {
  inboxId: string
  serverTimeMs: number
  lists: List[]
  tasks: Task[]
  tags: Tag[]
}

export type DocKind = 'habit' | 'habit_checkin' | 'focus' | 'prefs' | 'summary_template' | 'comment'

export interface Doc<T = unknown> {
  id: string
  kind: DocKind
  body: T
  updatedMs: number
}

// ---- inputs ----------------------------------------------------------

export interface NewTask {
  /** Client-chosen id: makes the create idempotent and lets the UI be optimistic. */
  id?: string
  listId: string
  parentId?: string | null
  title: string
  notes?: string
  kind?: TaskKind
  priority?: Priority
  startMs?: number | null
  dueMs?: number | null
  isAllDay?: boolean
  timeZone?: string
  reminders?: string[]
  repeatFlag?: string
  items?: ChecklistItem[]
  tags?: string[]
  /** Where to put it; absent appends to the end of the list. */
  sortOrder?: number
}

/** Absent leaves a field alone; `null` clears a nullable one. */
export interface TaskPatch {
  title?: string
  notes?: string
  kind?: TaskKind
  status?: Status
  priority?: Priority
  startMs?: number | null
  dueMs?: number | null
  isAllDay?: boolean
  timeZone?: string
  reminders?: string[]
  repeatFlag?: string
  exDates?: number[]
  items?: ChecklistItem[]
  tags?: string[]
  listId?: string
  sortOrder?: number
}

export interface NewList {
  id?: string
  name: string
  color?: string | null
}

export interface ListPatch {
  name?: string
  color?: string | null
  archived?: boolean
  viewMode?: ViewMode
  sortType?: string
  sortOrder?: number
}

export interface TagPatch {
  color?: string | null
  parent?: string | null
  sortOrder?: number
}

/**
 * From a line typed into the quick-add box to the task to create. Pure: the
 * caller supplies the time, the lists, and what the current view implies.
 */
import type { List, NewTask, Task } from '@/api/types'
import { parseQuickAdd, type Token } from '@/lib/nlp/parse'
import { startOfDay } from '@/lib/date'
import { firstSortOrder } from './organize'
import type { ViewSpec } from './organize'

export interface QuickAddContext {
  now: number
  lists: List[]
  inboxId: string
  view: ViewSpec
  /** Tasks already in the target list, to put the new one above them. */
  siblings: (listId: string) => Task[]
  /** Reminder given to a task with a time (`''` for none). */
  defaultReminder: string
}

export type QuickAddResult = { ok: true; input: NewTask } | { ok: false; reason: 'empty' }

/** The list a view adds to when the line names none. */
export function defaultListId(view: ViewSpec, inboxId: string): string {
  return view.kind === 'list' ? view.id : inboxId
}

/** `text` with the token spans blanked, except those `keep` accepts. */
function withoutTokens(text: string, tokens: Token[], keep: (t: Token) => boolean): string {
  let out = ''
  let at = 0
  for (const t of tokens) {
    out += text.slice(at, t.start)
    if (keep(t)) out += text.slice(t.start, t.end)
    at = t.end
  }
  return (out + text.slice(at)).replace(/\s+/g, ' ').trim()
}

export function buildQuickAdd(text: string, cx: QuickAddContext): QuickAddResult {
  const parsed = parseQuickAdd(text, cx.now)
  const named = parsed.listName === null ? null : cx.lists.find((l) => !l.archived && l.name.toLowerCase() === parsed.listName!.toLowerCase())
  const listId = named?.id ?? defaultListId(cx.view, cx.inboxId)
  // A `~name` that matches no list is just text: the user did not mean to file it anywhere.
  const title = parsed.listName !== null && !named ? withoutTokens(text, parsed.tokens, (t) => t.kind === 'list') : parsed.title
  if (!title) return { ok: false, reason: 'empty' }

  let dueMs = parsed.dueMs
  let isAllDay = parsed.isAllDay
  // Adding from Today or Next 7 Days with no date of its own means "today", so it shows up where it was added.
  if (dueMs === null && (cx.view.kind === 'today' || cx.view.kind === 'week')) {
    dueMs = startOfDay(cx.now)
    isAllDay = true
  }
  const tags = [...parsed.tags]
  if (cx.view.kind === 'tag') {
    const viewTag = cx.view.name
    if (!tags.some((t) => t.toLowerCase() === viewTag.toLowerCase())) tags.push(viewTag)
  }

  const input: NewTask = {
    listId,
    title,
    tags,
    sortOrder: firstSortOrder(cx.siblings(listId)),
  }
  if (dueMs !== null) {
    input.dueMs = dueMs
    input.isAllDay = isAllDay
    if (!isAllDay && cx.defaultReminder) input.reminders = [cx.defaultReminder]
  }
  if (parsed.priority !== null) input.priority = parsed.priority
  return { ok: true, input }
}

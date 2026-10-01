/**
 * Search over tasks and lists, run in the browser on the data already loaded.
 * (The server's own full-text search matches whole words; the search box wants
 * "grocer" to find "groceries" as you type, so it does its own substring match.)
 */
import type { List, Task } from '@/api/types'

export type SearchMode = 'task' | 'list'

export const MAX_RESULTS = 50

/** A piece of text and whether it matched the query, for drawing highlights. */
export interface Segment {
  text: string
  hit: boolean
}

/** Split `text` around every case-insensitive occurrence of `query`. */
export function highlight(text: string, query: string): Segment[] {
  const q = query.trim().toLowerCase()
  if (!q) return [{ text, hit: false }]
  const lower = text.toLowerCase()
  const out: Segment[] = []
  let at = 0
  for (let i = lower.indexOf(q); i >= 0; i = lower.indexOf(q, at)) {
    if (i > at) out.push({ text: text.slice(at, i), hit: false })
    out.push({ text: text.slice(i, i + q.length), hit: true })
    at = i + q.length
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false })
  return out
}

export interface TaskHit {
  task: Task
  /** A short piece of the notes around the match, when the title did not match. */
  snippet: string | null
}

/** A window of `text` around the first match of `query`. */
export function snippet(text: string, query: string, radius = 40): string {
  const i = text.toLowerCase().indexOf(query.trim().toLowerCase())
  if (i < 0) return text.slice(0, radius * 2)
  const start = Math.max(0, i - radius)
  const end = Math.min(text.length, i + query.trim().length + radius)
  return `${start > 0 ? '…' : ''}${text.slice(start, end).replace(/\s+/g, ' ')}${end < text.length ? '…' : ''}`
}

/**
 * Tasks whose title or notes contain every word of `query` (any order), best
 * first: title matches before notes-only ones, open before completed, then the
 * most recently changed. Trashed tasks never match. An empty query lists the
 * most recently changed tasks.
 */
export function searchTasks(query: string, tasks: Task[], limit = MAX_RESULTS): TaskHit[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const live = tasks.filter((t) => t.deletedMs === null)
  if (words.length === 0) {
    return [...live].sort((a, b) => b.updatedMs - a.updatedMs).slice(0, 8).map((task) => ({ task, snippet: null }))
  }
  const hits: { task: Task; inTitle: boolean; snippet: string | null }[] = []
  for (const task of live) {
    const title = task.title.toLowerCase()
    const body = `${title}\n${task.notes.toLowerCase()}`
    if (!words.every((w) => body.includes(w))) continue
    const inTitle = words.every((w) => title.includes(w))
    hits.push({ task, inTitle, snippet: inTitle || !task.notes ? null : snippet(task.notes, words.find((w) => task.notes.toLowerCase().includes(w)) ?? words[0]!) })
  }
  hits.sort((a, b) => Number(b.inTitle) - Number(a.inTitle) || Number(a.task.status === 'done') - Number(b.task.status === 'done') || b.task.updatedMs - a.task.updatedMs)
  return hits.slice(0, limit).map(({ task, snippet: s }) => ({ task, snippet: s }))
}

/** Lists whose name contains `query`, in the sidebar's order. An empty query lists them all. */
export function searchLists(query: string, lists: List[], limit = MAX_RESULTS): List[] {
  const q = query.trim().toLowerCase()
  return lists
    .filter((l) => !q || l.name.toLowerCase().includes(q))
    .sort((a, b) => a.sortOrder - b.sortOrder)
    .slice(0, limit)
}

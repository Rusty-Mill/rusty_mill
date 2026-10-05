/** Flatten groups into the rows the list draws, and pick which of them are on screen. */
import type { List, Tag, Task } from '@/api/types'
import type { Filter } from '../filters/logic'
import type { Group, ViewSpec } from './organize'

export const HEADER_HEIGHT = 36
export const ROW_HEIGHT = 40
/** Above this many rows the list renders only the visible window. */
export const VIRTUALIZE_ABOVE = 200

export type Row =
  | { type: 'header'; key: string; label: string; count: number; tone?: 'overdue'; collapsed: boolean }
  | { type: 'task'; task: Task; groupKey: string }

/** A group per header (skipped for the single unlabelled group), its tasks unless collapsed. */
export function buildRows(groups: Group[], collapsed: Record<string, boolean>, viewKey: string): Row[] {
  const rows: Row[] = []
  for (const g of groups) {
    const isCollapsed = !!collapsed[`${viewKey}:${g.key}`]
    if (g.label) {
      const header: Row = { type: 'header', key: g.key, label: g.label, count: g.tasks.length, collapsed: isCollapsed }
      if (g.tone) header.tone = g.tone
      rows.push(header)
    }
    if (!isCollapsed) for (const task of g.tasks) rows.push({ type: 'task', task, groupKey: g.key })
  }
  return rows
}

export const rowHeight = (r: Row): number => (r.type === 'header' ? HEADER_HEIGHT : ROW_HEIGHT)

export interface Window {
  start: number
  end: number
  /** Height above the first drawn row, and the whole list's height. */
  offset: number
  total: number
}

/** The rows to draw for a scroll position: those in view plus `overscan` either side. */
export function windowRows(rows: Row[], scrollTop: number, viewport: number, overscan = 8): Window {
  const tops: number[] = []
  let total = 0
  for (const r of rows) {
    tops.push(total)
    total += rowHeight(r)
  }
  if (rows.length === 0) return { start: 0, end: 0, offset: 0, total: 0 }
  let first = 0
  while (first < rows.length - 1 && tops[first]! + rowHeight(rows[first]!) <= scrollTop) first++
  let last = first
  while (last < rows.length - 1 && tops[last]! < scrollTop + viewport) last++
  const start = Math.max(0, first - overscan)
  const end = Math.min(rows.length, last + 1 + overscan)
  return { start, end, offset: tops[start] ?? 0, total }
}

/** The title shown above a view. */
export function viewTitle(spec: ViewSpec, lists: List[], tags: Tag[], filters: Filter[], inboxId: string): string {
  switch (spec.kind) {
    case 'all':
      return 'All'
    case 'today':
      return 'Today'
    case 'week':
      return 'Next 7 Days'
    case 'inbox':
      return lists.find((l) => l.id === inboxId)?.name ?? 'Inbox'
    case 'list':
      return lists.find((l) => l.id === spec.id)?.name ?? 'List'
    case 'tag':
      return tags.find((t) => t.name === spec.name)?.label ?? spec.name
    case 'filter':
      return filters.find((f) => f.id === spec.id)?.name ?? 'Filter'
  }
}

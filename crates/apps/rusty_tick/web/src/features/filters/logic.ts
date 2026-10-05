/** Saved filters: a name and a rule. Pure; no clock reads. Stored as `filter` client documents. */
import type { Priority, Task } from '@/api/types'

export type DateBucket = 'overdue' | 'today' | 'tomorrow' | 'next7' | 'later' | 'nodate'

/** Every field narrows the result; a field left empty does not. Within a field, any one match is enough. */
export interface FilterRule {
  lists: string[]
  tags: string[]
  priorities: Priority[]
  dates: DateBucket[]
}

export interface FilterBody {
  name: string
  rule: FilterRule
}

export interface Filter extends FilterBody {
  id: string
}

export const DATE_BUCKETS: [DateBucket, string][] = [
  ['overdue', 'Overdue'],
  ['today', 'Today'],
  ['tomorrow', 'Tomorrow'],
  ['next7', 'Next 7 Days'],
  ['later', 'Later'],
  ['nodate', 'No Date'],
]

export const PRIORITIES: [Priority, string][] = [
  [5, 'High'],
  [3, 'Medium'],
  [1, 'Low'],
  [0, 'None'],
]

export const emptyRule = (): FilterRule => ({ lists: [], tags: [], priorities: [], dates: [] })

/** Whether `task` passes `rule`; `bucket` is the task's due-date bucket (see `dateBucket`). */
export function matchesFilter(task: Task, rule: FilterRule, bucket: DateBucket): boolean {
  return (
    (rule.lists.length === 0 || rule.lists.includes(task.listId)) &&
    (rule.tags.length === 0 || task.tags.some((t) => rule.tags.includes(t))) &&
    (rule.priorities.length === 0 || rule.priorities.includes(task.priority)) &&
    (rule.dates.length === 0 || rule.dates.includes(bucket))
  )
}

const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])

/** A stored body as a filter body, or `null` if it is not one. Unknown values inside the rule are dropped. */
export function asFilterBody(body: unknown): FilterBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as { name?: unknown; rule?: Record<string, unknown> | null }
  if (typeof o.name !== 'string' || o.name.trim() === '') return null
  const r = o.rule ?? {}
  const buckets = DATE_BUCKETS.map(([b]) => b)
  const prios = PRIORITIES.map(([p]) => p)
  return {
    name: o.name,
    rule: {
      lists: strings(r.lists),
      tags: strings(r.tags),
      priorities: Array.isArray(r.priorities) ? r.priorities.filter((p): p is Priority => prios.includes(p as Priority)) : [],
      dates: strings(r.dates).filter((d): d is DateBucket => buckets.includes(d as DateBucket)),
    },
  }
}

/** What the Summary page is asked to produce, and the (de)serialising of the remembered choice. */
import type { Priority } from '@/api/types'
import { dayKey } from '@/lib/date'
import { RANGE_KEYS, type CustomRange, type RangeKey } from './range'

export const TEMPLATES = ['daily', 'weekly', 'simple'] as const
export type TemplateId = (typeof TEMPLATES)[number]
export const TEMPLATE_LABELS: Record<TemplateId, string> = { daily: 'Daily report', weekly: 'Weekly report', simple: 'Simple list' }

export type StatusFilter = 'completed' | 'open' | 'all'
export type GroupMode = 'none' | 'list'

export interface SummaryOptions {
  template: TemplateId
  range: RangeKey
  custom: CustomRange
  /** Empty means every list. */
  listIds: string[]
  status: StatusFilter
  /** Empty means every priority. */
  priorities: Priority[]
  /** Empty means every tag (and untagged tasks). */
  tags: string[]
  groupBy: GroupMode
  showList: boolean
  showDue: boolean
  showPriority: boolean
  showTags: boolean
  showCompletedTime: boolean
}

export function defaultOptions(now: number): SummaryOptions {
  const today = dayKey(now)
  return {
    template: 'weekly',
    range: 'thisWeek',
    custom: { from: today, to: today },
    listIds: [],
    status: 'all',
    priorities: [],
    tags: [],
    groupBy: 'none',
    showList: true,
    showDue: true,
    showPriority: false,
    showTags: false,
    showCompletedTime: false,
  }
}

const isRecord = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null
const strings = (x: unknown): string[] => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string').slice(0, 200) : [])
const DAYKEY = /^\d{4}-\d{2}-\d{2}$/

/** Accept only well-formed values from a stored document; anything else falls back to the default. */
export function sanitizeOptions(input: unknown, now: number): SummaryOptions {
  const d = defaultOptions(now)
  if (!isRecord(input)) return d
  const bool = (k: keyof SummaryOptions): boolean => (typeof input[k] === 'boolean' ? (input[k] as boolean) : (d[k] as boolean))
  const custom = isRecord(input.custom) ? input.custom : {}
  return {
    template: TEMPLATES.find((t) => t === input.template) ?? d.template,
    range: RANGE_KEYS.find((r) => r === input.range) ?? d.range,
    custom: {
      from: typeof custom.from === 'string' && DAYKEY.test(custom.from) ? custom.from : d.custom.from,
      to: typeof custom.to === 'string' && DAYKEY.test(custom.to) ? custom.to : d.custom.to,
    },
    listIds: strings(input.listIds),
    status: input.status === 'completed' || input.status === 'open' || input.status === 'all' ? input.status : d.status,
    priorities: Array.isArray(input.priorities) ? input.priorities.filter((p): p is Priority => p === 0 || p === 1 || p === 3 || p === 5) : [],
    tags: strings(input.tags),
    groupBy: input.groupBy === 'list' ? 'list' : 'none',
    showList: bool('showList'),
    showDue: bool('showDue'),
    showPriority: bool('showPriority'),
    showTags: bool('showTags'),
    showCompletedTime: bool('showCompletedTime'),
  }
}

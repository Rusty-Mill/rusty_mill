/**
 * The subset of RFC 5545 `RRULE` the repeat picker produces: DAILY, WEEKLY
 * (with BYDAY), MONTHLY (with BYMONTHDAY) and YEARLY, each with an INTERVAL and
 * an optional UNTIL. Enough to advance a repeating task when it is completed.
 */
import { addDays, addMonths, startOfDay } from './date'

export type Freq = 'DAILY' | 'WEEKLY' | 'MONTHLY' | 'YEARLY'
const DAY_CODES = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'] as const
const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

export interface Rule {
  freq: Freq
  interval: number
  /** Weekdays as `Date#getDay()` numbers, for WEEKLY. */
  byDay: number[]
  /** Day of month, for MONTHLY. */
  byMonthDay: number | null
  /** Last allowed occurrence (Unix ms), if the rule ends. */
  until: number | null
}

/** `null` for an empty or unsupported rule. */
export function parseRule(text: string): Rule | null {
  const body = text.trim().replace(/^RRULE:/i, '')
  if (!body) return null
  const parts = new Map<string, string>()
  for (const piece of body.split(';')) {
    const [k, v] = piece.split('=')
    if (k && v) parts.set(k.toUpperCase(), v.toUpperCase())
  }
  const freq = parts.get('FREQ')
  if (freq !== 'DAILY' && freq !== 'WEEKLY' && freq !== 'MONTHLY' && freq !== 'YEARLY') return null
  const interval = Number(parts.get('INTERVAL') ?? '1')
  if (!Number.isInteger(interval) || interval < 1) return null
  const byDay = (parts.get('BYDAY') ?? '')
    .split(',')
    .map((c) => DAY_CODES.indexOf(c as (typeof DAY_CODES)[number]))
    .filter((i) => i >= 0)
  const monthDay = parts.get('BYMONTHDAY')
  const until = parts.get('UNTIL')
  return {
    freq,
    interval,
    byDay: [...new Set(byDay)].sort((a, b) => a - b),
    byMonthDay: monthDay ? Number(monthDay) : null,
    until: until ? parseUntil(until) : null,
  }
}

function parseUntil(text: string): number | null {
  const m = /^(\d{4})(\d{2})(\d{2})/.exec(text)
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 23, 59, 59).getTime() : null
}

/** The `RRULE` text for a rule, the inverse of `parseRule`. */
export function formatRule(rule: Rule): string {
  const parts = [`FREQ=${rule.freq}`]
  if (rule.interval > 1) parts.push(`INTERVAL=${rule.interval}`)
  if (rule.freq === 'WEEKLY' && rule.byDay.length) parts.push(`BYDAY=${rule.byDay.map((d) => DAY_CODES[d]).join(',')}`)
  if (rule.freq === 'MONTHLY' && rule.byMonthDay) parts.push(`BYMONTHDAY=${rule.byMonthDay}`)
  return `RRULE:${parts.join(';')}`
}

/**
 * The occurrence after `from` (keeping its time of day), or `null` when the
 * rule is empty, unsupported, or has ended.
 */
export function nextOccurrence(ruleText: string, from: number): number | null {
  const rule = parseRule(ruleText)
  if (!rule) return null
  const next = advance(rule, from)
  return rule.until !== null && next > rule.until ? null : next
}

function advance(rule: Rule, from: number): number {
  switch (rule.freq) {
    case 'DAILY':
      return addDays(from, rule.interval)
    case 'WEEKLY':
      return rule.byDay.length ? nextWeekday(rule, from) : addDays(from, 7 * rule.interval)
    case 'MONTHLY': {
      const shifted = addMonths(from, rule.interval)
      if (!rule.byMonthDay) return shifted
      const d = new Date(shifted)
      const last = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate()
      d.setDate(Math.min(rule.byMonthDay, last))
      return d.getTime()
    }
    case 'YEARLY':
      return addMonths(from, 12 * rule.interval)
  }
}

/** The next listed weekday after `from`; past the last one, the first of the interval-th week on. */
function nextWeekday(rule: Rule, from: number): number {
  const today = new Date(from).getDay()
  const later = rule.byDay.find((d) => d > today)
  if (later !== undefined) return addDays(from, later - today)
  const first = rule.byDay[0] ?? today
  return addDays(from, 7 * rule.interval - today + first)
}

/** `Every day`, `Every 2 weeks on Mon, Wed`, `Every month on the 15th`… */
export function describeRule(ruleText: string): string {
  const rule = parseRule(ruleText)
  if (!rule) return 'Does not repeat'
  const unit = { DAILY: 'day', WEEKLY: 'week', MONTHLY: 'month', YEARLY: 'year' }[rule.freq]
  const every = rule.interval === 1 ? `Every ${unit}` : `Every ${rule.interval} ${unit}s`
  if (rule.freq === 'WEEKLY' && rule.byDay.length) return `${every} on ${rule.byDay.map((d) => DAY_NAMES[d]).join(', ')}`
  if (rule.freq === 'MONTHLY' && rule.byMonthDay) return `${every} on the ${ordinal(rule.byMonthDay)}`
  return every
}

function ordinal(n: number): string {
  const s = ['th', 'st', 'nd', 'rd']
  const v = n % 100
  return `${n}${s[(v - 20) % 10] ?? s[v] ?? s[0]}`
}

/**
 * Advance a repeating task's dates to its next occurrence, or `null` when it
 * does not repeat or the rule has ended. A start date moves with the due date.
 */
export function advanceDates(
  task: { repeatFlag: string; dueMs: number | null; startMs: number | null },
  exDates: number[] = [],
): { dueMs: number; startMs: number | null } | null {
  if (task.dueMs === null) return null
  let due: number | null = task.dueMs
  const skipped = new Set(exDates.map(startOfDay))
  // Skip occurrences the user excluded; the guard bounds a malformed rule.
  for (let i = 0; i < 400 && due !== null; i++) {
    due = nextOccurrence(task.repeatFlag, due)
    if (due === null || !skipped.has(startOfDay(due))) break
  }
  if (due === null) return null
  const shift = due - task.dueMs
  return { dueMs: due, startMs: task.startMs === null ? null : task.startMs + shift }
}

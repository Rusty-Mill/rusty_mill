/**
 * The date ranges the Summary page offers. A range is half-open,
 * `[startMs, endMs)`, so a task at exactly midnight belongs to the day that
 * starts then and never to two ranges at once.
 */
import { addDays, addMonths, monthName, parseDayKey, startOfDay, startOfWeek, type WeekStart } from '@/lib/date'

export const RANGE_KEYS = ['today', 'yesterday', 'thisWeek', 'lastWeek', 'thisMonth', 'lastMonth', 'custom'] as const
export type RangeKey = (typeof RANGE_KEYS)[number]

export const RANGE_LABELS: Record<RangeKey, string> = {
  today: 'Today',
  yesterday: 'Yesterday',
  thisWeek: 'This week',
  lastWeek: 'Last week',
  thisMonth: 'This month',
  lastMonth: 'Last month',
  custom: 'Custom',
}

/** Inclusive `YYYY-MM-DD` day keys, as an `<input type="date">` produces them. */
export interface CustomRange {
  from: string
  to: string
}

export interface DateRange {
  startMs: number
  /** Exclusive: local midnight after the last day. */
  endMs: number
}

const startOfMonth = (ms: number): number => {
  const d = new Date(ms)
  return new Date(d.getFullYear(), d.getMonth(), 1).getTime()
}

/** The range for `key`, or `null` for a custom range that is missing or not a real date. */
export function computeRange(key: RangeKey, now: number, weekStart: WeekStart, custom?: CustomRange): DateRange | null {
  const today = startOfDay(now)
  switch (key) {
    case 'today':
      return { startMs: today, endMs: addDays(today, 1) }
    case 'yesterday':
      return { startMs: addDays(today, -1), endMs: today }
    case 'thisWeek': {
      const start = startOfWeek(now, weekStart)
      return { startMs: start, endMs: addDays(start, 7) }
    }
    case 'lastWeek': {
      const start = startOfWeek(now, weekStart)
      return { startMs: addDays(start, -7), endMs: start }
    }
    case 'thisMonth': {
      const start = startOfMonth(now)
      return { startMs: start, endMs: startOfMonth(addMonths(start, 1)) }
    }
    case 'lastMonth': {
      const start = startOfMonth(now)
      return { startMs: startOfMonth(addMonths(start, -1)), endMs: start }
    }
    case 'custom': {
      const a = custom ? parseDayKey(custom.from) : null
      const b = custom ? parseDayKey(custom.to) : null
      if (a === null || b === null) return null
      const [lo, hi] = a <= b ? [a, b] : [b, a] // a reversed pair is a slip, not an error
      return { startMs: lo, endMs: addDays(hi, 1) }
    }
  }
}

export const inRange = (ms: number | null, r: DateRange): boolean => ms !== null && ms >= r.startMs && ms < r.endMs

/** `Sep 29` for one day, `Sep 28 – Oct 4` for several; the year is added when it is not the current one. */
export function formatRange(r: DateRange, now: number): string {
  const first = new Date(r.startMs)
  const last = new Date(addDays(r.endMs, -1))
  const year = new Date(now).getFullYear()
  const day = (d: Date, withYear: boolean): string => `${monthName(d.getMonth())} ${d.getDate()}${withYear ? `, ${d.getFullYear()}` : ''}`
  if (startOfDay(r.startMs) === startOfDay(last.getTime())) return day(first, first.getFullYear() !== year)
  const withYear = first.getFullYear() !== year || last.getFullYear() !== year
  return `${day(first, withYear)} – ${day(last, withYear)}`
}

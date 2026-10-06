/** Countdowns to important dates. Pure; stored as `countdown` client documents. */
import { diffDays, parseDayKey } from '@/lib/date'

export interface CountdownBody {
  name: string
  /** Local midnight of the target day. */
  dateMs: number
}

export function asCountdownBody(body: unknown): CountdownBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.name !== 'string' || o.name.trim() === '' || typeof o.dateMs !== 'number' || !Number.isFinite(o.dateMs)) return null
  return { name: o.name, dateMs: o.dateMs }
}

/** A `YYYY-MM-DD` form value as a countdown body, or `null` for a blank name or a bad date. */
export function fromForm(name: string, day: string): CountdownBody | null {
  const dateMs = parseDayKey(day)
  return name.trim() === '' || dateMs === null ? null : { name: name.trim(), dateMs }
}

/** Whole days from today to the target: negative once it has passed. */
export const daysLeft = (dateMs: number, now: number): number => diffDays(now, dateMs)

export function describe(days: number): string {
  if (days === 0) return 'Today'
  const n = Math.abs(days)
  return `${n} ${n === 1 ? 'day' : 'days'} ${days > 0 ? 'left' : 'ago'}`
}

/** Upcoming first (soonest at the top), then past ones, most recent first. */
export function byUpcoming<T extends CountdownBody>(items: readonly T[], now: number): T[] {
  const rank = (i: T): number => {
    const d = daysLeft(i.dateMs, now)
    return d >= 0 ? d : 1e9 - d
  }
  return [...items].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name))
}

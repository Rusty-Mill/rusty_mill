/**
 * When reminders should fire. Pure: give it the tasks and the time, get back
 * what is due next; the component owns the timers and the notification.
 */
import type { Task } from '@/api/types'
import { addDays, atTime, dayKey, startOfDay } from '@/lib/date'
import { checkinId, isDue, parseGoal, type Checkin, type Habit } from '@/features/habits/logic'

export interface Due {
  key: string // what and when: the same reminder is never shown twice
  title: string
  /** Notification text under the title. */
  body: string
  atMs: number
}

const UNIT_MS = { D: 86_400_000, H: 3_600_000, M: 60_000, S: 1000 } as const
const TRIGGER = /^TRIGGER:(-?)P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/

/** Milliseconds a `TRIGGER:` moves from the due time (negative = before); null if unreadable. */
export function triggerOffsetMs(trigger: string): number | null {
  const m = TRIGGER.exec(trigger)
  if (!m) return null
  const ms = Number(m[2] ?? 0) * UNIT_MS.D + Number(m[3] ?? 0) * UNIT_MS.H + Number(m[4] ?? 0) * UNIT_MS.M + Number(m[5] ?? 0) * UNIT_MS.S
  return m[1] ? -ms : ms
}

/** Reminders of open, untrashed tasks that fall after `now` and within `horizonMs`, soonest first. */
export function upcoming(tasks: Iterable<Task>, now: number, horizonMs: number): Due[] {
  const out: Due[] = []
  for (const t of tasks) {
    if (t.status !== 'open' || t.deletedMs !== null || t.dueMs === null) continue
    for (const trigger of t.reminders) {
      const offset = triggerOffsetMs(trigger)
      if (offset === null) continue
      const atMs = t.dueMs + offset
      if (atMs > now && atMs <= now + horizonMs) out.push({ key: `${t.id}|${trigger}|${atMs}`, title: t.title, body: 'Task reminder', atMs })
    }
  }
  return out.sort((a, b) => a.atMs - b.atMs)
}

/** Habit reminders (`HH:MM` each day) that fall after `now` within `horizonMs`, for habits due that day and not yet checked in. */
export function upcomingHabits(habits: Habit[], checkins: Record<string, Checkin>, now: number, horizonMs: number): Due[] {
  const out: Due[] = []
  for (const h of habits) {
    const m = h.archived || h.reminder === null ? null : /^(\d{2}):(\d{2})$/.exec(h.reminder)
    if (!m) continue
    for (let day = startOfDay(now); day <= now + horizonMs; day = addDays(day, 1)) {
      const atMs = atTime(day, Number(m[1]), Number(m[2]))
      if (atMs <= now || atMs > now + horizonMs || !isDue(h, day) || (checkins[checkinId(h.id, dayKey(day))]?.count ?? 0) >= parseGoal(h.goal).count) continue
      out.push({ key: `habit|${h.id}|${atMs}`, title: h.name, body: 'Habit reminder', atMs })
    }
  }
  return out.sort((a, b) => a.atMs - b.atMs)
}

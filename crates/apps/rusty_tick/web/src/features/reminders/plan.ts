/**
 * When reminders should fire. Pure: give it the tasks and the time, get back
 * what is due next; the component owns the timers and the notification.
 */
import type { Task } from '@/api/types'

export interface Due {
  key: string // task, reminder and time: the same reminder is never shown twice
  taskId: string
  title: string
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
      if (atMs > now && atMs <= now + horizonMs) out.push({ key: `${t.id}|${trigger}|${atMs}`, taskId: t.id, title: t.title, atMs })
    }
  }
  return out.sort((a, b) => a.atMs - b.atMs)
}

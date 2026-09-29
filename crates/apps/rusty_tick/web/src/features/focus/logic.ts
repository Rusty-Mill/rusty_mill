/** Pure focus-timer maths: no clock reads, no React. Callers pass `now`. */
import { addDays, dayKey, diffDays, monthName, startOfDay } from '@/lib/date'

export type FocusMode = 'pomo' | 'stopwatch'

export interface FocusRecordBody {
  startMs: number
  endMs: number
  durationSec: number
  kind: FocusMode
  taskId: string | null
  taskTitle: string | null
}

export interface FocusRecord extends FocusRecordBody {
  id: string
}

export interface FocusSettings {
  focusMin: number
  shortMin: number
  longMin: number
  /** Pomos before a long break. */
  longEvery: number
}

export const DEFAULT_SETTINGS: FocusSettings = { focusMin: 25, shortMin: 5, longMin: 15, longEvery: 4 }

const clampInt = (v: unknown, min: number, max: number, fallback: number): number =>
  typeof v === 'number' && Number.isFinite(v) ? Math.min(max, Math.max(min, Math.round(v))) : fallback

export function sanitizeSettings(input: unknown): FocusSettings {
  const o = (typeof input === 'object' && input !== null ? input : {}) as Record<string, unknown>
  return {
    focusMin: clampInt(o.focusMin, 1, 180, DEFAULT_SETTINGS.focusMin),
    shortMin: clampInt(o.shortMin, 1, 60, DEFAULT_SETTINGS.shortMin),
    longMin: clampInt(o.longMin, 1, 120, DEFAULT_SETTINGS.longMin),
    longEvery: clampInt(o.longEvery, 2, 12, DEFAULT_SETTINGS.longEvery),
  }
}

/** A stored focus doc body is a record only if it has the record's fields (settings share the kind). */
export function asRecordBody(body: unknown): FocusRecordBody | null {
  if (typeof body !== 'object' || body === null) return null
  const o = body as Record<string, unknown>
  if (typeof o.startMs !== 'number' || typeof o.endMs !== 'number' || typeof o.durationSec !== 'number') return null
  if (o.kind !== 'pomo' && o.kind !== 'stopwatch') return null
  return {
    startMs: o.startMs,
    endMs: o.endMs,
    durationSec: o.durationSec,
    kind: o.kind,
    taskId: typeof o.taskId === 'string' ? o.taskId : null,
    taskTitle: typeof o.taskTitle === 'string' ? o.taskTitle : null,
  }
}

/** `25:00`, or `1:05:00` once an hour is reached (stopwatch). */
export function formatClock(totalSec: number): string {
  const s = Math.max(0, Math.floor(totalSec))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const pad = (n: number): string => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`
}

/** `1h 25m`, `25m`, `45s`; whole minutes once there is at least one. */
export function formatDuration(totalSec: number): string {
  const s = Math.max(0, Math.round(totalSec))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m`
  return s > 0 ? `${s}s` : '0m'
}

export interface FocusStats {
  todayPomos: number
  todaySec: number
  totalPomos: number
  totalSec: number
}

export function computeStats(records: readonly FocusRecordBody[], now: number): FocusStats {
  const dayStart = startOfDay(now)
  const dayEnd = startOfDay(addDays(dayStart, 1))
  const out: FocusStats = { todayPomos: 0, todaySec: 0, totalPomos: 0, totalSec: 0 }
  for (const r of records) {
    const pomo = r.kind === 'pomo' ? 1 : 0
    out.totalPomos += pomo
    out.totalSec += r.durationSec
    if (r.endMs >= dayStart && r.endMs < dayEnd) {
      out.todayPomos += pomo
      out.todaySec += r.durationSec
    }
  }
  return out
}

/** Pomos finished today: decides when the next break is the long one. */
export const pomosToday = (records: readonly FocusRecordBody[], now: number): number => computeStats(records, now).todayPomos

export function breakFor(pomoCount: number, s: FocusSettings): { kind: 'short' | 'long'; sec: number } {
  const long = pomoCount > 0 && pomoCount % s.longEvery === 0
  return long ? { kind: 'long', sec: s.longMin * 60 } : { kind: 'short', sec: s.shortMin * 60 }
}

export interface DayGroup {
  key: string
  label: string
  records: FocusRecord[]
}

/** Newest first, grouped by the local day a session ended. */
export function groupByDay(records: readonly FocusRecord[], now: number): DayGroup[] {
  const sorted = [...records].sort((a, b) => b.endMs - a.endMs)
  const groups: DayGroup[] = []
  for (const r of sorted) {
    const key = dayKey(r.endMs)
    const last = groups[groups.length - 1]
    if (last && last.key === key) last.records.push(r)
    else groups.push({ key, label: dayLabel(r.endMs, now), records: [r] })
  }
  return groups
}

function dayLabel(ms: number, now: number): string {
  const d = diffDays(now, ms)
  if (d === 0) return 'Today'
  if (d === -1) return 'Yesterday'
  const date = new Date(ms)
  const base = `${monthName(date.getMonth())} ${date.getDate()}`
  return date.getFullYear() === new Date(now).getFullYear() ? base : `${base}, ${date.getFullYear()}`
}

// ---- the running session ------------------------------------------------

export interface Session {
  mode: FocusMode
  phase: 'focus' | 'break'
  /** Countdown length; `null` for a stopwatch. */
  targetSec: number | null
  startedMs: number
  /** Set while paused. */
  pausedAt: number | null
  /** Time spent paused so far, excluding the current pause. */
  pausedTotalMs: number
  taskId: string | null
  taskTitle: string | null
}

/** Milliseconds actually spent running: timestamps, not ticks, so a throttled tab stays right. */
export function elapsedMs(s: Session, now: number): number {
  return Math.max(0, (s.pausedAt ?? now) - s.startedMs - s.pausedTotalMs)
}

/** Seconds left on a countdown (never negative); `null` for a stopwatch. */
export function remainingSec(s: Session, now: number): number | null {
  return s.targetSec === null ? null : Math.max(0, Math.ceil((s.targetSec * 1000 - elapsedMs(s, now)) / 1000))
}

export const isFinished = (s: Session, now: number): boolean => s.targetSec !== null && elapsedMs(s, now) >= s.targetSec * 1000

/** Fraction of the ring that is filled, 0..1. A stopwatch fills once a minute. */
export function ringProgress(s: Session | null, mode: FocusMode, now: number): number {
  if (!s) return mode === 'pomo' ? 1 : 0
  if (s.targetSec === null) return (elapsedMs(s, now) % 60_000) / 60_000
  return Math.min(1, Math.max(0, 1 - elapsedMs(s, now) / (s.targetSec * 1000)))
}

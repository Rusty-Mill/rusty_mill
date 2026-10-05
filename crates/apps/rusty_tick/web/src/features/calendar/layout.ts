/**
 * Pure calendar logic: which tasks land where, and the geometry to draw them.
 * All day arithmetic goes through `@/lib/date` so a DST week is still seven
 * columns; the time grid is laid out in wall-clock minutes for the same reason
 * (the hour that does not exist on a spring-forward day just has no block in it).
 */
import type { Task } from '@/api/types'
import { CALENDAR_MODES, type CalendarMode } from '@/app/paths'
import { addDays, addMonths, atTime, diffDays, formatTime, monthGrid, startOfDay, startOfWeek, MINUTE, type WeekStart } from '@/lib/date'
import { laterOccurrences } from '@/lib/recurrence'

export const DEFAULT_COLOR = '#4772fa'
/** A timed task with no end is drawn as a block this long. */
export const DEFAULT_DURATION_MIN = 30
export const AGENDA_DAYS = 30

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const short = (s: string): string => s.slice(0, 3)

export const parseMode = (s: string | undefined): CalendarMode => ((CALENDAR_MODES as readonly string[]).includes(s ?? '') ? (s as CalendarMode) : 'm')

// ---- events ----------------------------------------------------------

export interface CalEvent {
  task: Task
  /** `span`: all-day or multi-day, drawn as a bar. `timed`: inside one day, drawn as a block. */
  kind: 'span' | 'timed'
  /** Local midnight of the first and last day covered (inclusive). */
  startDay: number
  endDay: number
  /** For a timed event the block; for a span the task's own start and due. */
  startMs: number
  endMs: number
  done: boolean
  /** A later occurrence of a repeating task, shown but not movable: only the next one is the task's own. */
  projected: boolean
}

/** The event a task draws as, or `null` if it has no place on a calendar. */
export function toEvent(task: Task, showDone = false): CalEvent | null {
  if (task.deletedMs !== null || task.dueMs === null) return null
  const done = task.status !== 'open'
  if (done && !showDone) return null
  const due = task.dueMs
  // A start after the due date is bad data, not a reversed span.
  const start = task.startMs !== null && task.startMs <= due ? task.startMs : due
  const startDay = startOfDay(start)
  let endDay = startOfDay(due)
  // A timed task ending exactly at midnight belongs to the day it ended, not the next one.
  if (!task.isAllDay && due === endDay && due > start) endDay = startOfDay(due - 1)
  if (task.isAllDay || startDay !== endDay) return { task, kind: 'span', startDay, endDay: Math.max(startDay, endDay), startMs: start, endMs: due, done, projected: false }
  const end = due > start ? due : start + DEFAULT_DURATION_MIN * MINUTE
  return { task, kind: 'timed', startDay, endDay, startMs: start, endMs: end, done, projected: false }
}

/** Events for `tasks`; with `until`, repeating tasks also show their later occurrences before it. */
export function buildEvents(tasks: Iterable<Task>, showDone = false, until?: number): CalEvent[] {
  const out: CalEvent[] = []
  for (const t of tasks) {
    const e = toEvent(t, showDone)
    if (!e) continue
    out.push(e)
    if (until === undefined || t.status !== 'open' || !t.repeatFlag) continue
    for (const o of laterOccurrences(t, until, t.exDates)) {
      const later = toEvent({ ...t, dueMs: o.dueMs, startMs: o.startMs })
      if (later) out.push({ ...later, projected: true })
    }
  }
  return out
}

/** Bars first (longest first), then timed by time; ties by title so the order is stable. */
export function compareEvents(a: CalEvent, b: CalEvent): number {
  if (a.kind !== b.kind) return a.kind === 'span' ? -1 : 1
  if (a.kind === 'span' && a.startDay !== b.startDay) return a.startDay - b.startDay
  const lenA = a.endDay - a.startDay
  const lenB = b.endDay - b.startDay
  if (a.kind === 'span' && lenA !== lenB) return lenB - lenA
  return a.startMs - b.startMs || a.task.title.localeCompare(b.task.title) || a.task.id.localeCompare(b.task.id)
}

export const coversDay = (e: CalEvent, day: number): boolean => e.startDay <= day && day <= e.endDay

/** Events covering `day`, in display order. */
export function eventsOnDay(events: CalEvent[], day: number): CalEvent[] {
  return events.filter((e) => coversDay(e, day)).sort(compareEvents)
}

// ---- bars per week row ------------------------------------------------

export interface Segment {
  event: CalEvent
  /** Column indexes into the row's days, inclusive. */
  startCol: number
  endCol: number
  lane: number
  /** The event runs on past the row's edge (drawn with a square end). */
  cutStart: boolean
  cutEnd: boolean
}

/**
 * Bars for one row of days, each in the first lane that is free for its whole
 * width, so bars never overlap and a multi-day bar keeps its lane across days.
 */
export function weekSegments(events: CalEvent[], days: number[]): { segments: Segment[]; lanes: number } {
  const first = days[0]
  const last = days[days.length - 1]
  if (first === undefined || last === undefined) return { segments: [], lanes: 0 }
  const inRow = events.filter((e) => e.endDay >= first && e.startDay <= last).sort(compareEvents)
  const laneEnds: number[] = [] // last occupied column per lane
  const segments: Segment[] = []
  for (const event of inRow) {
    const startCol = Math.max(0, diffDays(first, event.startDay))
    const endCol = Math.min(days.length - 1, diffDays(first, event.endDay))
    let lane = laneEnds.findIndex((end) => end < startCol)
    if (lane < 0) lane = laneEnds.length
    laneEnds[lane] = endCol
    segments.push({ event, startCol, endCol, lane, cutStart: event.startDay < first, cutEnd: event.endDay > last })
  }
  return { segments, lanes: laneEnds.length }
}

/** Per column, how many bars sit in a lane at or beyond `maxLanes` (the "+N more"). */
export function hiddenPerColumn(segments: Segment[], columns: number, maxLanes: number): number[] {
  const out = new Array<number>(columns).fill(0)
  for (const s of segments) {
    if (s.lane < maxLanes) continue
    for (let c = s.startCol; c <= s.endCol; c++) out[c] = (out[c] ?? 0) + 1
  }
  return out
}

export const chunkWeeks = (days: number[]): number[][] => Array.from({ length: Math.ceil(days.length / 7) }, (_, i) => days.slice(i * 7, i * 7 + 7))

// ---- time grid --------------------------------------------------------

export interface TimedBlock {
  event: CalEvent
  /** Percent of the day column. */
  top: number
  height: number
  /** Side-by-side placement among events that overlap in time. */
  col: number
  cols: number
  startMin: number
  endMin: number
}

export const MINUTES_PER_DAY = 1440

/** Wall-clock minutes since local midnight. */
export function minutesOfDay(ms: number): number {
  const d = new Date(ms)
  return d.getHours() * 60 + d.getMinutes()
}

/** Blocks for one day: overlapping events share the width in columns. */
export function layoutTimed(events: CalEvent[], day: number): TimedBlock[] {
  const nextDay = addDays(day, 1)
  const items = events
    .filter((e) => e.kind === 'timed' && e.startDay === day)
    .map((event) => {
      const startMin = minutesOfDay(event.startMs)
      // Clipped at midnight: a block never spills into the next column.
      const endMin = event.endMs >= nextDay ? MINUTES_PER_DAY : Math.max(minutesOfDay(event.endMs), startMin + 1)
      return { event, startMin, endMin }
    })
    .sort((a, b) => a.startMin - b.startMin || b.endMin - a.endMin || a.event.task.id.localeCompare(b.event.task.id))

  const out: TimedBlock[] = []
  let cluster: { event: CalEvent; startMin: number; endMin: number; col: number }[] = []
  let colEnds: number[] = []
  let clusterEnd = -1
  const flush = (): void => {
    for (const c of cluster) {
      out.push({ ...c, top: (c.startMin / MINUTES_PER_DAY) * 100, height: ((c.endMin - c.startMin) / MINUTES_PER_DAY) * 100, cols: colEnds.length })
    }
    cluster = []
    colEnds = []
  }
  for (const it of items) {
    if (cluster.length > 0 && it.startMin >= clusterEnd) flush()
    let col = colEnds.findIndex((end) => end <= it.startMin)
    if (col < 0) col = colEnds.length
    colEnds[col] = it.endMin
    clusterEnd = cluster.length === 0 ? it.endMin : Math.max(clusterEnd, it.endMin)
    cluster.push({ ...it, col })
  }
  flush()
  return out
}

/** The time `minutes` past midnight on `day`, snapped down to a multiple of `step`. */
export function slotTime(day: number, minutes: number, step = 15): number {
  const m = Math.max(0, Math.min(MINUTES_PER_DAY - step, Math.floor(minutes / step) * step))
  return atTime(day, Math.floor(m / 60), m % 60)
}

// ---- rescheduling -----------------------------------------------------

export interface Reschedule {
  dueMs: number
  startMs?: number | null
  isAllDay: boolean
}

/** Move by whole days, keeping the time of day and the length. */
export function moveByDays(task: Task, days: number): Reschedule {
  const out: Reschedule = { dueMs: addDays(task.dueMs ?? 0, days), isAllDay: task.isAllDay }
  if (task.startMs !== null) out.startMs = addDays(task.startMs, days)
  return out
}

/** Drop onto a time slot: becomes timed, its start moves to `slotMs`, its length is kept. */
export function moveToSlot(task: Task, slotMs: number): Reschedule {
  const due = task.dueMs ?? slotMs
  const start = task.startMs !== null && task.startMs <= due ? task.startMs : due
  if (task.isAllDay) return task.startMs === null ? { dueMs: slotMs, isAllDay: false } : { dueMs: slotMs, startMs: null, isAllDay: false }
  const out: Reschedule = { dueMs: due + (slotMs - start), isAllDay: false }
  if (task.startMs !== null) out.startMs = slotMs
  return out
}

/** Drop onto a day in the all-day strip: the start lands on `day`, a multi-day span keeps its length. */
export function moveToAllDay(task: Task, day: number): Reschedule {
  const due = task.dueMs ?? day
  const start = task.startMs !== null && task.startMs <= due ? task.startMs : due
  const delta = diffDays(start, day)
  const out: Reschedule = { dueMs: startOfDay(addDays(due, delta)), isAllDay: true }
  if (task.startMs !== null) out.startMs = startOfDay(start) === startOfDay(due) ? null : startOfDay(addDays(start, delta))
  return out
}

// ---- range, stepping, titles -----------------------------------------

export interface Range {
  /** Local midnight of the first day. */
  start: number
  /** Local midnight after the last day (exclusive). */
  end: number
  days: number[]
}

export function visibleRange(mode: CalendarMode, anchor: number, weekStart: WeekStart): Range {
  let days: number[]
  if (mode === 'm') days = monthGrid(anchor, weekStart)
  else if (mode === 'w') {
    const s = startOfWeek(anchor, weekStart)
    days = Array.from({ length: 7 }, (_, i) => addDays(s, i))
  } else if (mode === 'd') days = [startOfDay(anchor)]
  else days = Array.from({ length: AGENDA_DAYS }, (_, i) => addDays(startOfDay(anchor), i))
  const first = days[0] ?? startOfDay(anchor)
  const last = days[days.length - 1] ?? first
  return { start: first, end: addDays(last, 1), days }
}

/** The anchor after moving one page (`dir` is 1 or -1) in `mode`. */
export function stepAnchor(mode: CalendarMode, anchor: number, dir: 1 | -1): number {
  if (mode === 'm') return addMonths(anchor, dir)
  if (mode === 'w') return addDays(anchor, 7 * dir)
  if (mode === 'd') return addDays(anchor, dir)
  return addDays(anchor, AGENDA_DAYS * dir)
}

const md = (ms: number): string => `${short(MONTHS[new Date(ms).getMonth()] ?? '')} ${new Date(ms).getDate()}`
const y = (ms: number): number => new Date(ms).getFullYear()

/** `September 2026`, `Sep 27 – Oct 3, 2026`, `Tuesday, Sep 29, 2026`. */
export function rangeTitle(mode: CalendarMode, anchor: number, range: Range): string {
  const a = new Date(anchor)
  if (mode === 'm') return `${MONTHS[a.getMonth()]} ${a.getFullYear()}`
  if (mode === 'd') return `${WEEKDAYS[a.getDay()]}, ${md(anchor)}, ${a.getFullYear()}`
  const first = range.days[0] ?? anchor
  const last = range.days[range.days.length - 1] ?? first
  if (y(first) !== y(last)) return `${md(first)}, ${y(first)} – ${md(last)}, ${y(last)}`
  return `${md(first)} – ${new Date(first).getMonth() === new Date(last).getMonth() && mode === 'w' ? new Date(last).getDate() : md(last)}, ${y(last)}`
}

// ---- labels -----------------------------------------------------------

/** `Tuesday, September 29, 2026` */
export function longDate(ms: number): string {
  const d = new Date(ms)
  return `${WEEKDAYS[d.getDay()]}, ${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`
}

export const cellLabel = (day: number, count: number): string => `${longDate(day)}, ${count} ${count === 1 ? 'task' : 'tasks'}`

/** `Tue, Sep 29` */
export function shortDate(ms: number): string {
  const d = new Date(ms)
  return `${short(WEEKDAYS[d.getDay()] ?? '')}, ${md(ms)}`
}

export const weekdayShort = (i: number): string => short(WEEKDAYS[i] ?? '')

/** When an event happens, for a popover: `Tue, Sep 29, 3:00 PM`, `Sep 28 – Sep 30`. */
export function eventWhen(e: CalEvent, hour12: boolean): string {
  const t = e.task
  if (e.kind === 'timed') {
    const s = formatTime(e.startMs, hour12)
    return t.startMs !== null && e.endMs > e.startMs ? `${shortDate(e.startDay)}, ${s} – ${formatTime(e.endMs, hour12)}` : `${shortDate(e.startDay)}, ${s}`
  }
  if (e.startDay === e.endDay) return shortDate(e.startDay)
  const at = (ms: number, day: number): string => (t.isAllDay ? shortDate(day) : `${shortDate(day)}, ${formatTime(ms, hour12)}`)
  return `${at(e.startMs, e.startDay)} – ${at(e.endMs, e.endDay)}`
}

/** The short time prefix of a bar in the month grid, or `''` for all-day and multi-day. */
export const barTime = (e: CalEvent, hour12: boolean): string => (e.kind === 'timed' ? formatTime(e.startMs, hour12) : '')

/** Days in `range` with at least one event, each with its events. */
export function agendaGroups(events: CalEvent[], range: Range): { day: number; events: CalEvent[] }[] {
  return range.days.map((day) => ({ day, events: eventsOnDay(events, day) })).filter((g) => g.events.length > 0)
}

/** Open tasks whose day has passed, oldest first: what the agenda lists above today. */
export function overdueEvents(events: CalEvent[], today: number): CalEvent[] {
  return events.filter((e) => !e.done && !e.projected && e.endDay < today).sort((a, b) => a.endDay - b.endDay || a.task.title.localeCompare(b.task.title))
}

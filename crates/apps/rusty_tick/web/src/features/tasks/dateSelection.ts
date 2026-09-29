/**
 * The date popover's rules, apart from the popover: what each quick button
 * means, and how a chosen day, time, reminder and repeat become task fields.
 */
import type { Task } from '@/api/types'
import { addDays, atTime, HOUR, startOfDay, startOfWeek, type WeekStart } from '@/lib/date'
import { formatRule, parseRule, type Freq } from '@/lib/recurrence'

export interface DateFields {
  dueMs: number | null
  startMs: number | null
  isAllDay: boolean
  reminders: string[]
  repeatFlag: string
}

export const fieldsOf = (t: Pick<Task, 'dueMs' | 'startMs' | 'isAllDay' | 'reminders' | 'repeatFlag'>): DateFields => ({
  dueMs: t.dueMs,
  startMs: t.startMs,
  isAllDay: t.isAllDay,
  reminders: t.reminders,
  repeatFlag: t.repeatFlag,
})

export const CLEARED: DateFields = { dueMs: null, startMs: null, isAllDay: false, reminders: [], repeatFlag: '' }

export interface QuickDate {
  id: 'today' | 'tomorrow' | 'nextWeek' | 'evening'
  label: string
  /** Local midnight of the day, plus a time for "Later this evening". */
  day: number
  time: { h: number; m: number } | null
}

/** The four quick buttons. "Later this evening" is 18:00, or an hour from now if that is later still. */
export function quickDates(now: number, weekStart: WeekStart): QuickDate[] {
  const today = startOfDay(now)
  const evening = Math.max(atTime(today, 18), now + HOUR)
  const nextWeek = addDays(startOfWeek(now, weekStart), 7)
  const rounded = new Date(evening)
  return [
    { id: 'today', label: 'Today', day: today, time: null },
    { id: 'tomorrow', label: 'Tomorrow', day: addDays(today, 1), time: null },
    { id: 'nextWeek', label: 'Next week', day: nextWeek, time: null },
    { id: 'evening', label: 'Later this evening', day: startOfDay(evening), time: { h: rounded.getHours(), m: rounded.getMinutes() >= 30 ? 30 : 0 } },
  ]
}

/** Reminder choices. Times are relative to the due time, or to 09:00 for an all-day task. */
export const REMINDERS_TIMED = [
  { value: '', label: 'None' },
  { value: 'TRIGGER:PT0S', label: 'At time of event' },
  { value: 'TRIGGER:-PT5M', label: '5 minutes before' },
  { value: 'TRIGGER:-PT30M', label: '30 minutes before' },
  { value: 'TRIGGER:-PT1H', label: '1 hour before' },
  { value: 'TRIGGER:-P1D', label: '1 day before' },
] as const

export const REMINDERS_ALL_DAY = [
  { value: '', label: 'None' },
  { value: 'TRIGGER:PT9H', label: 'On the day (09:00)' },
  { value: 'TRIGGER:-PT15H', label: '1 day before (09:00)' },
  { value: 'TRIGGER:-PT39H', label: '2 days before (09:00)' },
] as const

export const reminderOptions = (allDay: boolean) => (allDay ? REMINDERS_ALL_DAY : REMINDERS_TIMED)

/** The reminder a fresh time should get, if the user's default applies to it. */
export function reminderForNewTime(defaultReminder: string): string[] {
  return defaultReminder ? [defaultReminder] : []
}

export type RepeatPreset = 'none' | 'daily' | 'weekly' | 'monthly' | 'yearly' | 'custom'

/** The `RRULE` for a preset, anchored on `due` (its weekday or day of month). */
export function repeatRule(preset: RepeatPreset, due: number, custom?: { freq: Freq; interval: number }): string {
  const d = new Date(due)
  switch (preset) {
    case 'none':
      return ''
    case 'daily':
      return formatRule({ freq: 'DAILY', interval: 1, byDay: [], byMonthDay: null, until: null })
    case 'weekly':
      return formatRule({ freq: 'WEEKLY', interval: 1, byDay: [d.getDay()], byMonthDay: null, until: null })
    case 'monthly':
      return formatRule({ freq: 'MONTHLY', interval: 1, byDay: [], byMonthDay: d.getDate(), until: null })
    case 'yearly':
      return formatRule({ freq: 'YEARLY', interval: 1, byDay: [], byMonthDay: null, until: null })
    case 'custom': {
      const c = custom ?? { freq: 'DAILY' as const, interval: 2 }
      return formatRule({ freq: c.freq, interval: Math.max(1, Math.floor(c.interval)), byDay: c.freq === 'WEEKLY' ? [d.getDay()] : [], byMonthDay: c.freq === 'MONTHLY' ? d.getDate() : null, until: null })
    }
  }
}

/** Which preset a stored rule corresponds to. */
export function presetOf(rule: string): RepeatPreset {
  const r = parseRule(rule)
  if (!r) return 'none'
  const plain = r.interval === 1
  if (r.freq === 'DAILY') return plain ? 'daily' : 'custom'
  if (r.freq === 'WEEKLY') return plain && r.byDay.length <= 1 ? 'weekly' : 'custom'
  if (r.freq === 'MONTHLY') return plain ? 'monthly' : 'custom'
  return plain ? 'yearly' : 'custom'
}

export interface Selection {
  /** Local midnight of the chosen due day, or `null` for no date. */
  day: number | null
  /** Time of day, or `null` for an all-day task. */
  time: { h: number; m: number } | null
  /** Start day for the Duration tab. */
  startDay: number | null
  startTime: { h: number; m: number } | null
  reminder: string
  repeat: string
}

/**
 * Turn the popover's state into task fields. No day means no date at all: a
 * time, reminder or repeat without a day would mean nothing, so they go too.
 * A start after the due date is corrected by making the start the due date.
 */
export function toFields(sel: Selection): DateFields {
  if (sel.day === null) return CLEARED
  const isAllDay = sel.time === null
  const dueMs = sel.time ? atTime(sel.day, sel.time.h, sel.time.m) : sel.day
  let startMs: number | null = null
  if (sel.startDay !== null) {
    startMs = sel.startTime ? atTime(sel.startDay, sel.startTime.h, sel.startTime.m) : sel.startDay
    if (startMs > dueMs) startMs = dueMs
  }
  const reminders = sel.reminder ? [sel.reminder] : []
  return { dueMs, startMs, isAllDay, reminders, repeatFlag: sel.repeat }
}

/** The popover's starting state for a task's current fields. */
export function selectionOf(f: DateFields): Selection {
  const clock = (ms: number): { h: number; m: number } => {
    const d = new Date(ms)
    return { h: d.getHours(), m: d.getMinutes() }
  }
  return {
    day: f.dueMs === null ? null : startOfDay(f.dueMs),
    time: f.dueMs === null || f.isAllDay ? null : clock(f.dueMs),
    startDay: f.startMs === null ? null : startOfDay(f.startMs),
    startTime: f.startMs === null || f.isAllDay ? null : clock(f.startMs),
    reminder: f.reminders[0] ?? '',
    repeat: f.repeatFlag,
  }
}

/**
 * Quick-add parsing: pulls dates, times, priority, `#tags` and a `~list` out of
 * a line of text. Pure: `now` is a parameter. The recognised tokens are
 * returned with their positions in the *original* text so the input can
 * highlight them while typing, and are removed from the title.
 *
 * Recognised (case-insensitive):
 *   dates      today · tomorrow · next mon|monday · monday (full names) · in 3 days|weeks · 2026-10-05
 *   times      5pm · 5:30pm · at 17:30 · at 5pm · 17:30
 *   priority   !high !medium !low !none · !1 (high) !2 (medium) !3 (low)
 *   tags       #tag
 *   list       ~list · ~"two words"
 * A date with no time is all-day. A time with no date means today, or
 * tomorrow if that time has already passed.
 */
import { addDays, atTime, parseDayKey, startOfDay } from '../date'
import type { Priority } from '@/api/types'

export type TokenKind = 'date' | 'time' | 'priority' | 'tag' | 'list'

export interface Token {
  kind: TokenKind
  /** Offsets into the original text, `end` exclusive. */
  start: number
  end: number
  text: string
}

export interface Parsed {
  /** What is left once the recognised tokens are removed. */
  title: string
  dueMs: number | null
  isAllDay: boolean
  priority: Priority | null
  /** As typed, without the `#`, in order, de-duplicated case-insensitively. */
  tags: string[]
  listName: string | null
  tokens: Token[]
}

const WEEKDAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday']
const weekdayIndex = (word: string): number => WEEKDAYS.findIndex((d) => d === word || (word.length === 3 && d.startsWith(word)))

const PRIORITY_WORDS: Record<string, Priority> = { high: 5, h: 5, '1': 5, medium: 3, med: 3, m: 3, '2': 3, low: 1, l: 1, '3': 1, none: 0, '0': 0 }

interface Span {
  start: number
  end: number
}

export function parseQuickAdd(input: string, now: number): Parsed {
  // `work` is `input` with matched spans blanked, so later rules cannot re-match them
  // and offsets stay valid.
  let work = input
  const tokens: Token[] = []
  const take = (kind: TokenKind, m: RegExpExecArray, span: Span = { start: m.index, end: m.index + m[0].length }): void => {
    tokens.push({ kind, start: span.start, end: span.end, text: input.slice(span.start, span.end) })
    work = work.slice(0, span.start) + ' '.repeat(span.end - span.start) + work.slice(span.end)
  }
  /** Every match of `re` in the current `work`. Sigil rules require a word start. */
  const all = (re: RegExp): RegExpExecArray[] => [...work.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : re.flags + 'g'))]

  const tags: string[] = []
  let listName: string | null = null
  let priority: Priority | null = null
  let dueDay: number | null = null
  let time: { h: number; m: number } | null = null
  let relative: number | null = null // `in 3 hours`: an absolute instant

  // Sigils first: their bodies must not be mistaken for dates ("#today").
  for (const m of all(/(?<![^\s])~(?:"([^"]+)"|(\S+))/)) {
    listName ??= (m[1] ?? m[2])!
    take('list', m)
  }
  for (const m of all(/(?<![^\s])#([\p{L}\p{N}_-]+)/u)) {
    const tag = m[1]!
    if (!tags.some((t) => t.toLowerCase() === tag.toLowerCase())) tags.push(tag)
    take('tag', m)
  }
  for (const m of all(/(?<![^\s])!(high|medium|med|low|none|h|m|l|[0-3])(?![\p{L}\p{N}])/iu)) {
    const p = PRIORITY_WORDS[m[1]!.toLowerCase()]
    if (p === undefined) continue
    priority = p
    take('priority', m)
  }

  for (const m of all(/\bin\s+(\d{1,3})\s*(minutes?|mins?|hours?|hrs?|days?|weeks?)\b/i)) {
    const n = Number(m[1])
    const unit = m[2]!.toLowerCase()
    if (unit.startsWith('m')) relative = now + n * 60_000
    else if (unit.startsWith('h')) relative = now + n * 3_600_000
    else dueDay = startOfDay(addDays(now, unit.startsWith('w') ? 7 * n : n))
    take('date', m)
  }
  for (const m of all(/\bnext\s+(sun|mon|tue|wed|thu|fri|sat)[a-z]*\b/i)) {
    const target = weekdayIndex(m[1]!.toLowerCase())
    if (target < 0) continue
    // "next mon" is always in the future: the coming one, or a week later if that is today.
    const ahead = ((target - new Date(now).getDay() + 7) % 7) || 7
    dueDay = startOfDay(addDays(now, ahead))
    take('date', m)
  }
  for (const m of all(/\b(\d{4}-\d{2}-\d{2})\b/)) {
    const day = parseDayKey(m[1]!)
    if (day === null) continue
    dueDay = day
    take('date', m)
  }
  for (const m of all(/\b(sunday|monday|tuesday|wednesday|thursday|friday|saturday)\b/i)) {
    const target = weekdayIndex(m[1]!.toLowerCase())
    const ahead = (target - new Date(now).getDay() + 7) % 7
    dueDay = startOfDay(addDays(now, ahead))
    take('date', m)
  }
  for (const m of all(/\b(today|tomorrow)\b/i)) {
    dueDay = startOfDay(addDays(now, m[1]!.toLowerCase() === 'today' ? 0 : 1))
    take('date', m)
  }

  // Times: `at 5pm`, `5:30pm`, `17:30`.
  for (const m of all(/\b(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b/i)) {
    const t = clock(Number(m[1]), Number(m[2] ?? 0), m[3]!.toLowerCase())
    if (!t) continue
    time = t
    take('time', m)
  }
  for (const m of all(/\b(?:at\s+)?([01]?\d|2[0-3]):([0-5]\d)\b/i)) {
    time = { h: Number(m[1]), m: Number(m[2]) }
    take('time', m)
  }

  let dueMs: number | null = null
  let isAllDay = false
  if (relative !== null) {
    dueMs = relative
  } else if (dueDay !== null) {
    isAllDay = time === null
    dueMs = time ? atTime(dueDay, time.h, time.m) : dueDay
  } else if (time) {
    const today = atTime(startOfDay(now), time.h, time.m)
    dueMs = today > now ? today : addDays(today, 1)
  }

  tokens.sort((a, b) => a.start - b.start)
  return {
    title: work.replace(/\s+/g, ' ').trim(),
    dueMs,
    isAllDay,
    priority,
    tags,
    listName,
    tokens,
  }
}

function clock(hour: number, minute: number, meridiem: string): { h: number; m: number } | null {
  if (hour < 1 || hour > 12 || minute > 59) return null
  return { h: (hour % 12) + (meridiem === 'pm' ? 12 : 0), m: minute }
}

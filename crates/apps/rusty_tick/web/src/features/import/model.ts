/**
 * The shape every importer reads its source into, and the small readers they
 * share. Pure. A source never writes; `applyImport` does that once for all.
 */
import type { Priority, Status } from '@/api/types'
import { parseRule } from '@/lib/recurrence'

export interface ImportedTask {
  /** Stable within the source (its own id, or one made from the row): seeds the task id, so importing twice adds nothing. */
  key: string
  /** The key the first importer (#821) gave this row, for sources whose keys changed since; see `legacyKeys`. */
  legacyKey: string | null
  /** List name as the source calls it; `''` means the Inbox. */
  list: string
  parentKey: string | null
  title: string
  notes: string
  priority: Priority
  startMs: number | null
  dueMs: number | null
  isAllDay: boolean
  timeZone: string
  reminders: string[]
  repeatFlag: string
  tags: string[]
  items: { title: string; done: boolean }[]
  status: Status
}

export interface ImportResult {
  /** Human name of the source, for messages: `TickTick`, `Todoist`, ... */
  source: string
  tasks: ImportedTask[]
  /** Rows that were not tasks, or had no title. */
  skipped: number
  /** Tasks whose date or repeat rule could not be read: the text was kept in the notes, or the repeat dropped. */
  lossy: number
}

export const blankTask = (key: string, title: string): ImportedTask => ({
  key,
  legacyKey: null,
  list: '',
  parentKey: null,
  title,
  notes: '',
  priority: 0,
  startMs: null,
  dueMs: null,
  isAllDay: false,
  timeZone: '',
  reminders: [],
  repeatFlag: '',
  tags: [],
  items: [],
  status: 'open',
})

/** Column lookup by header name, case- and space-insensitive, trying each alias in turn. */
export function columns(header: string[]): (row: string[], ...names: string[]) => string {
  const index = new Map(header.map((h, i) => [h.trim().toLowerCase(), i]))
  return (row, ...names) => {
    for (const n of names) {
      const i = index.get(n)
      if (i !== undefined) return (row[i] ?? '').trim()
    }
    return ''
  }
}

export interface When {
  ms: number
  allDay: boolean
}

/** The calendar day `ms` falls on in `zone` (or locally), as local midnight: the app's all-day convention. */
function localDay(ms: number, zone: string): number {
  if (zone) {
    try {
      const [y, m, d] = new Intl.DateTimeFormat('en-CA', { timeZone: zone, year: 'numeric', month: '2-digit', day: '2-digit' }).format(ms).split('-').map(Number)
      return new Date(y!, m! - 1, d!).getTime()
    } catch {
      /* unknown zone: use the local day */
    }
  }
  const at = new Date(ms)
  return new Date(at.getFullYear(), at.getMonth(), at.getDate()).getTime()
}

const daysIn = (y: number, mo: number): number => new Date(Date.UTC(y, mo, 0)).getUTCDate()

/** Offset minutes east of UTC for `Z`, `±hh`mm or `±hh:mm`; `null` past ±14:00 or 59 minutes. */
function readOffset(text: string): number | null {
  if (text === 'Z') return 0
  const [h, mi] = [Number(text.slice(1, 3)), Number(text.slice(-2))]
  if (h > 14 || mi > 59 || (h === 14 && mi > 0)) return null
  return (text[0] === '-' ? -1 : 1) * (h * 60 + mi)
}

/**
 * An ISO-like date: `2026-10-05` (all day), or a date and time with `T` or a
 * space, an optional `Z` or `±hhmm`/`±hh:mm` offset (none means local time).
 * `allDay` forces a dated time onto its calendar day in `zone`. `null` when
 * unreadable or out of range (`2026-02-30`, `25:00`): a deadline is never invented.
 */
export function parseWhen(text: string, allDay = false, zone = ''): When | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?(Z|[+-]\d{2}:?\d{2})?)?$/.exec(text.trim())
  if (!m) return null
  const [y, mo, d, h, mi, s] = [m[1], m[2], m[3], m[4] ?? '0', m[5] ?? '0', m[6] ?? '0'].map(Number) as [number, number, number, number, number, number]
  if (mo < 1 || mo > 12 || d < 1 || d > daysIn(y, mo) || h > 23 || mi > 59 || s > 59) return null
  if (m[4] === undefined) return { ms: new Date(y, mo - 1, d).getTime(), allDay: true }
  let ms: number
  if (m[7] === undefined) ms = new Date(y, mo - 1, d, h, mi, s).getTime()
  else {
    const off = readOffset(m[7])
    if (off === null) return null
    ms = Date.UTC(y, mo - 1, d, h, mi, s) - off * 60_000
  }
  return allDay ? { ms: localDay(ms, zone), allDay: true } : { ms, allDay: false }
}

/** Append `label: text` to `notes`, so a value the app could not read is kept, not lost. */
export const keepInNotes = (notes: string, label: string, text: string): string => [notes, `${label}: ${text}`].filter(Boolean).join('\n\n')

/** The `[label, text]` of each date that was given but could not be read. */
export const unreadDates = (dates: [label: string, text: string, when: When | null][]): [string, string][] =>
  dates.filter(([, text, when]) => text !== '' && when === null).map(([label, text]) => [label, text])

/** An `RRULE` the app understands, as `RRULE:...`; `''` for none or one it does not. */
export function readRepeat(rule: string): string {
  const body = rule.trim().replace(/^RRULE:/i, '')
  return body && parseRule(body) ? `RRULE:${body}` : ''
}

const PRIORITY_WORDS: Record<string, Priority> = { high: 5, medium: 3, med: 3, low: 1, none: 0 }

/** `0|1|3|5` or `high|medium|low|none`; anything else is no priority. */
export function readPriority(text: string): Priority {
  const t = text.trim().toLowerCase()
  if (t in PRIORITY_WORDS) return PRIORITY_WORDS[t]!
  return ([0, 1, 3, 5] as const).find((p) => String(p) === t) ?? 0
}

/** Split a tag cell on commas (and spaces after a `#`), dropping blanks and `#`. */
export const readTags = (text: string): string[] =>
  text
    .split(/,|\s+(?=#)/)
    .map((t) => t.trim().replace(/^#/, ''))
    .filter(Boolean)

/**
 * Keys made from content collide when a file holds the same row twice. Each
 * key encodes its parts and the occurrence count as JSON, so no title (not
 * even one ending in `#2` or holding `|`) can collide with another row, and a
 * re-import of the same file yields the same keys.
 */
export function uniqueKeys(): (...parts: string[]) => string {
  const seen = new Map<string, number>()
  return (...parts) => {
    const base = JSON.stringify(parts)
    const n = (seen.get(base) ?? 0) + 1
    seen.set(base, n)
    return JSON.stringify([...parts, n])
  }
}

/**
 * The keys #821's importer gave content-keyed rows, in file order: `base`,
 * then `base#2`, `base#3`, ... for repeats. That scheme can give two rows the
 * same key (`Task`, `Task`, `Task#2` give `Task#2` twice); which of them owns
 * the task #821 created is decided by `applyImport`, in #821's write order.
 */
export function legacyKeys(): (base: string) => string {
  const seen = new Map<string, number>()
  return (base) => {
    const n = (seen.get(base) ?? 0) + 1
    seen.set(base, n)
    return n === 1 ? base : `${base}#${n}`
  }
}

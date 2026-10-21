/**
 * Read an iCalendar (.ics) file into tasks. Pure. VEVENTs and VTODOs become one
 * task each; cancelled and completed ones are skipped. Repeat rules outside the
 * subset the app understands (`parseRule`) are dropped and counted, not guessed at.
 *
 * Exceptions to a repeat rule are kept: `EXDATE`s, and the occurrence a moved
 * or cancelled instance (`RECURRENCE-ID`) replaces, become the master's
 * `exDates`, so the original occurrence is not shown beside the moved one. An
 * exception that cannot be read is counted, not guessed at.
 */
import { addDays, startOfDay } from './date'
import { parseRule } from './recurrence'

/** The most `exDates` the server keeps on a task (`MAX_EX_DATES` in `service.rs`); a longer list is refused. */
export const MAX_EX_DATES = 1000

/**
 * Exceptions as a task stores them: unique, ascending, and at most
 * `MAX_EX_DATES`, so a write is never refused. Past the cap the latest are
 * kept: a long-running series' recent and future exceptions are the ones the
 * calendar shows.
 */
export const storedExDates = (exDates: readonly number[]): number[] => [...new Set(exDates)].sort((a, b) => a - b).slice(-MAX_EX_DATES)

export interface IcsTask {
  /** The feed's `UID`, or one made from the title and dates when it has none: stable across fetches. */
  uid: string
  title: string
  notes: string
  startMs: number | null
  dueMs: number | null
  isAllDay: boolean
  /** IANA zone the dates were written in, or `''` for UTC and floating times. */
  timeZone: string
  repeatFlag: string
  /** The original start (Unix ms) of the occurrence this instance replaces; `null` for a master or a one-off. */
  recurrenceId: number | null
  /** Occurrences left out of the repeat rule, on the due-date timeline the app's `exDates` use. */
  exDates: number[]
}

export interface IcsResult {
  tasks: IcsTask[]
  /** Items skipped as cancelled or completed. */
  skipped: number
  /** Items whose repeat rule was not understood and so were imported once. */
  droppedRepeats: number
  /** Exceptions (`EXDATE`, `RECURRENCE-ID`) that could not be read, so an occurrence may show that the source left out. */
  droppedExceptions: number
}

interface Prop {
  params: Map<string, string>
  value: string
}

const unfold = (text: string): string[] => text.replace(/\r?\n[ \t]/g, '').split(/\r?\n/)

const unescapeText = (v: string): string => v.replace(/\\([nN,;\\])/g, (_, c: string) => (c === 'n' || c === 'N' ? '\n' : c))

function parseLine(line: string): [string, Prop] | null {
  const colon = line.indexOf(':')
  if (colon < 1) return null
  const [name, ...rest] = line.slice(0, colon).split(';')
  const params = new Map<string, string>()
  for (const p of rest) {
    const eq = p.indexOf('=')
    if (eq > 0) params.set(p.slice(0, eq).toUpperCase(), p.slice(eq + 1).replace(/^"|"$/g, ''))
  }
  return [name!.toUpperCase(), { params, value: line.slice(colon + 1) }]
}

/** Offset (ms) of `zone` from UTC at the instant `ms`; throws RangeError for an unknown zone. */
function zoneOffset(ms: number, zone: string): number {
  const parts = new Intl.DateTimeFormat('en-US', { timeZone: zone, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' }).formatToParts(ms)
  const n = (t: string): number => Number(parts.find((p) => p.type === t)?.value)
  return Date.UTC(n('year'), n('month') - 1, n('day'), n('hour'), n('minute'), n('second')) - Math.floor(ms / 1000) * 1000
}

/** The UTC instant at which a wall clock in `zone` reads the given fields. */
function zonedToUtc(f: number[], zone: string): number {
  const wall = Date.UTC(f[0]!, f[1]! - 1, f[2]!, f[3]!, f[4]!, f[5]!)
  const first = wall - zoneOffset(wall, zone)
  return wall - zoneOffset(first, zone) // once more: the offset may differ across a DST change
}

interface When {
  ms: number
  allDay: boolean
  zone: string
}

function parseWhen(p: Prop): When | null {
  const m = /^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})(Z)?)?$/.exec(p.value.trim())
  if (!m) return null
  const f = [m[1], m[2], m[3], m[4] ?? '0', m[5] ?? '0', m[6] ?? '0'].map(Number)
  if (m[4] === undefined || p.params.get('VALUE') === 'DATE') return { ms: new Date(f[0]!, f[1]! - 1, f[2]!).getTime(), allDay: true, zone: '' }
  if (m[7]) return { ms: Date.UTC(f[0]!, f[1]! - 1, f[2]!, f[3]!, f[4]!, f[5]!), allDay: false, zone: '' }
  const zone = p.params.get('TZID') ?? ''
  if (zone) {
    try {
      return { ms: zonedToUtc(f, zone), allDay: false, zone }
    } catch {
      /* unknown zone: read it as local time */
    }
  }
  return { ms: new Date(f[0]!, f[1]! - 1, f[2]!, f[3]!, f[4]!, f[5]!).getTime(), allDay: false, zone: '' }
}

/** One VEVENT or VTODO: its first value of each property, and every `EXDATE` line. */
interface Item {
  props: Map<string, Prop>
  exdates: Prop[]
  isTodo: boolean
}

/** Each instant an `EXDATE` line names (comma-separated, sharing its parameters); `null` for one that cannot be read. */
const exdateValues = (p: Prop): (When | null)[] => p.value.split(',').map((value) => parseWhen({ params: p.params, value }))

export function parseIcs(text: string): IcsResult {
  const items: Item[] = []
  let item: Item | null = null
  for (const line of unfold(text)) {
    const upper = line.trim().toUpperCase()
    if (upper === 'BEGIN:VEVENT' || upper === 'BEGIN:VTODO') {
      item = { props: new Map(), exdates: [], isTodo: upper === 'BEGIN:VTODO' }
      continue
    }
    if (upper === 'END:VEVENT' || upper === 'END:VTODO') {
      if (item) items.push(item)
      item = null
      continue
    }
    const prop = item && parseLine(line)
    if (!prop || !item) continue
    if (prop[0] === 'EXDATE') item.exdates.push(prop[1])
    else if (!item.props.has(prop[0])) item.props.set(prop[0], prop[1]) // first one wins; nested alarms repeat names
  }
  const out: IcsResult = { tasks: [], skipped: 0, droppedRepeats: 0, droppedExceptions: 0 }
  const replaced: { uid: string; ms: number }[] = [] // occurrences that moved or cancelled instances stand in for
  const shifts = new Map<IcsTask, number>() // master -> due minus start, to put an original start on the due timeline
  for (const it of items) add(out, it, replaced, shifts)
  for (const { uid, ms } of replaced) {
    const master = out.tasks.find((t) => t.uid === uid && t.recurrenceId === null && t.repeatFlag !== '')
    if (master) master.exDates.push(ms + shifts.get(master)!)
  }
  return out
}

function add(out: IcsResult, it: Item, replaced: { uid: string; ms: number }[], shifts: Map<IcsTask, number>): void {
  const p = it.props
  const rid = p.get('RECURRENCE-ID')
  const recurrence = rid ? parseWhen(rid) : null
  if (rid && !recurrence) out.droppedExceptions++
  const uidText = p.get('UID')?.value.trim() ?? ''
  if (recurrence && uidText) replaced.push({ uid: uidText, ms: recurrence.ms })

  const status = p.get('STATUS')?.value.trim().toUpperCase()
  if (status === 'CANCELLED' || status === 'COMPLETED') return void out.skipped++
  const title = unescapeText(p.get('SUMMARY')?.value ?? '').trim()
  const start = p.get('DTSTART') && parseWhen(p.get('DTSTART')!)
  const end = (p.get(it.isTodo ? 'DUE' : 'DTEND') && parseWhen(p.get(it.isTodo ? 'DUE' : 'DTEND')!)) || null
  if (!title || (!start && !end)) return void out.skipped++

  const allDay = (start ?? end)!.allDay
  let startMs: number | null = start ? start.ms : null
  let dueMs: number | null = end ? end.ms : startMs
  // An all-day DTEND is exclusive: the last day of the event is the day before it.
  if (allDay && !it.isTodo && end && start) dueMs = Math.max(start.ms, startOfDay(addDays(end.ms, -1)))
  const shift = start && dueMs !== null ? dueMs - start.ms : 0
  if (startMs !== null && startMs === dueMs) startMs = null // a point in time is just a due date

  const rrule = p.get('RRULE')?.value.trim() ?? ''
  const understood = rrule !== '' && parseRule(rrule) !== null
  if (rrule && !understood) out.droppedRepeats++
  const excluded = it.exdates.flatMap(exdateValues)
  out.droppedExceptions += excluded.filter((w) => w === null).length
  const task: IcsTask = {
    uid: uidText || `${title}|${startMs}|${dueMs}`,
    title,
    notes: unescapeText(p.get('DESCRIPTION')?.value ?? '').trim(),
    startMs,
    dueMs,
    isAllDay: allDay,
    timeZone: (start ?? end)!.zone,
    repeatFlag: understood ? `RRULE:${rrule.replace(/^RRULE:/i, '')}` : '',
    recurrenceId: recurrence ? recurrence.ms : null,
    exDates: understood ? excluded.flatMap((w) => (w ? [w.ms + shift] : [])) : [],
  }
  shifts.set(task, shift)
  out.tasks.push(task)
}

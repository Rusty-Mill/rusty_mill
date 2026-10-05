/**
 * Read an iCalendar (.ics) file into tasks. Pure. VEVENTs and VTODOs become one
 * task each; cancelled and completed ones are skipped. Repeat rules outside the
 * subset the app understands (`parseRule`) are dropped and counted, not guessed at.
 */
import { addDays, startOfDay } from './date'
import { parseRule } from './recurrence'

export interface IcsTask {
  title: string
  notes: string
  startMs: number | null
  dueMs: number | null
  isAllDay: boolean
  /** IANA zone the dates were written in, or `''` for UTC and floating times. */
  timeZone: string
  repeatFlag: string
}

export interface IcsResult {
  tasks: IcsTask[]
  /** Items skipped as cancelled or completed. */
  skipped: number
  /** Items whose repeat rule was not understood and so were imported once. */
  droppedRepeats: number
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

export function parseIcs(text: string): IcsResult {
  const out: IcsResult = { tasks: [], skipped: 0, droppedRepeats: 0 }
  let item: Map<string, Prop> | null = null
  for (const line of unfold(text)) {
    const upper = line.trim().toUpperCase()
    if (upper === 'BEGIN:VEVENT' || upper === 'BEGIN:VTODO') {
      item = new Map()
      continue
    }
    if (upper === 'END:VEVENT' || upper === 'END:VTODO') {
      if (item) add(out, item, upper === 'END:VTODO')
      item = null
      continue
    }
    const prop = item && parseLine(line)
    if (prop && item && !item.has(prop[0])) item.set(prop[0], prop[1]) // first one wins; nested alarms repeat names
  }
  return out
}

function add(out: IcsResult, p: Map<string, Prop>, isTodo: boolean): void {
  const status = p.get('STATUS')?.value.trim().toUpperCase()
  if (status === 'CANCELLED' || status === 'COMPLETED') return void out.skipped++
  const title = unescapeText(p.get('SUMMARY')?.value ?? '').trim()
  const start = p.get('DTSTART') && parseWhen(p.get('DTSTART')!)
  const end = (p.get(isTodo ? 'DUE' : 'DTEND') && parseWhen(p.get(isTodo ? 'DUE' : 'DTEND')!)) || null
  if (!title || (!start && !end)) return void out.skipped++

  const allDay = (start ?? end)!.allDay
  let startMs: number | null = start ? start.ms : null
  let dueMs: number | null = end ? end.ms : startMs
  // An all-day DTEND is exclusive: the last day of the event is the day before it.
  if (allDay && !isTodo && end && start) dueMs = Math.max(start.ms, startOfDay(addDays(end.ms, -1)))
  if (startMs !== null && startMs === dueMs) startMs = null // a point in time is just a due date

  const rrule = p.get('RRULE')?.value.trim() ?? ''
  const understood = rrule !== '' && parseRule(rrule) !== null
  if (rrule && !understood) out.droppedRepeats++
  out.tasks.push({
    title,
    notes: unescapeText(p.get('DESCRIPTION')?.value ?? '').trim(),
    startMs,
    dueMs,
    isAllDay: allDay,
    timeZone: (start ?? end)!.zone,
    repeatFlag: understood ? `RRULE:${rrule.replace(/^RRULE:/i, '')}` : '',
  })
}

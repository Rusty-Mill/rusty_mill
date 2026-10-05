import { describe, expect, it } from 'vitest'
import { parseIcs } from './ics'

const wrap = (...items: string[]): string => ['BEGIN:VCALENDAR', 'VERSION:2.0', ...items, 'END:VCALENDAR'].join('\r\n')
const event = (...lines: string[]): string => ['BEGIN:VEVENT', ...lines, 'END:VEVENT'].join('\r\n')

describe('parseIcs', () => {
  it('reads a UTC event as a start and a due time', () => {
    const { tasks } = parseIcs(wrap(event('SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'DTEND:20261005T093000Z')))
    expect(tasks).toEqual([
      { title: 'Standup', notes: '', startMs: Date.UTC(2026, 9, 5, 9), dueMs: Date.UTC(2026, 9, 5, 9, 30), isAllDay: false, timeZone: '', repeatFlag: '' },
    ])
  })

  it('treats an all-day DTEND as exclusive and a one-day event as a due date only', () => {
    const { tasks } = parseIcs(wrap(event('SUMMARY:Trip', 'DTSTART;VALUE=DATE:20261010', 'DTEND;VALUE=DATE:20261013'), event('SUMMARY:Holiday', 'DTSTART;VALUE=DATE:20261105', 'DTEND;VALUE=DATE:20261106')))
    expect(tasks[0]).toMatchObject({ isAllDay: true, startMs: new Date(2026, 9, 10).getTime(), dueMs: new Date(2026, 9, 12).getTime() })
    expect(tasks[1]).toMatchObject({ isAllDay: true, startMs: null, dueMs: new Date(2026, 10, 5).getTime() })
  })

  it('converts a TZID wall time to the right instant, DST included', () => {
    const { tasks } = parseIcs(wrap(event('SUMMARY:NY summer', 'DTSTART;TZID=America/New_York:20260705T120000'), event('SUMMARY:NY winter', 'DTSTART;TZID=America/New_York:20261205T120000')))
    expect(tasks[0]).toMatchObject({ dueMs: Date.UTC(2026, 6, 5, 16), timeZone: 'America/New_York' })
    expect(tasks[1]).toMatchObject({ dueMs: Date.UTC(2026, 11, 5, 17) })
  })

  it('unfolds long lines and unescapes text', () => {
    const { tasks } = parseIcs(wrap(event('SUMMARY:Plan\\, review', 'DESCRIPTION:line one\\nline\r\n  two\; end', 'DTSTART;VALUE=DATE:20261010')))
    expect(tasks[0]).toMatchObject({ title: 'Plan, review', notes: 'line one\nline two; end' })
  })

  it('keeps repeat rules the app understands and counts the ones it does not', () => {
    const { tasks, droppedRepeats } = parseIcs(wrap(event('SUMMARY:Weekly', 'DTSTART;VALUE=DATE:20261010', 'RRULE:FREQ=WEEKLY;BYDAY=MO'), event('SUMMARY:Odd', 'DTSTART;VALUE=DATE:20261010', 'RRULE:FREQ=SECONDLY')))
    expect(tasks[0]!.repeatFlag).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO')
    expect(tasks[1]!.repeatFlag).toBe('')
    expect(droppedRepeats).toBe(1)
  })

  it('imports a todo by its DUE date and skips cancelled, completed, untitled and undated items', () => {
    const todo = ['BEGIN:VTODO', 'SUMMARY:Pay rent', 'DUE:20261101T080000Z', 'BEGIN:VALARM', 'TRIGGER:-PT15M', 'END:VALARM', 'END:VTODO'].join('\r\n')
    const { tasks, skipped } = parseIcs(wrap(todo, event('SUMMARY:X', 'STATUS:CANCELLED', 'DTSTART:20261005T090000Z'), event('SUMMARY:Y', 'STATUS:COMPLETED', 'DTSTART:20261005T090000Z'), event('DTSTART:20261005T090000Z'), event('SUMMARY:Undated')))
    expect(tasks.map((t) => t.title)).toEqual(['Pay rent'])
    expect(tasks[0]).toMatchObject({ startMs: null, dueMs: Date.UTC(2026, 10, 1, 8) })
    expect(skipped).toBe(4)
  })

  it('returns nothing for text that is not a calendar', () => {
    expect(parseIcs('hello')).toEqual({ tasks: [], skipped: 0, droppedRepeats: 0 })
  })
})

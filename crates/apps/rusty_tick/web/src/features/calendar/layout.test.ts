import { describe, expect, it } from 'vitest'
import type { Task } from '@/api/types'
import { addDays, monthGrid, startOfDay } from '@/lib/date'
import {
  agendaGroups, buildEvents, cellLabel, chunkWeeks, eventWhen, eventsOnDay, hiddenPerColumn, layoutTimed, minutesOfDay, moveByDays, moveToAllDay, moveToSlot,
  overdueEvents, parseMode, rangeTitle, slotTime, stepAnchor, toEvent, visibleRange, weekSegments,
} from './layout'

const at = (y: number, mo: number, d: number, h = 0, mi = 0): number => new Date(y, mo - 1, d, h, mi).getTime()

let n = 0
function task(over: Partial<Task> = {}): Task {
  n++
  return {
    id: `t${n}`, listId: 'l', parentId: null, title: `Task ${n}`, notes: '', kind: 'text', status: 'open', priority: 0, startMs: null, dueMs: null, isAllDay: false,
    timeZone: 'America/Chicago', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [], sortOrder: 0, createdMs: 0, updatedMs: 0, completedMs: null, deletedMs: null, etag: '',
    ...over,
  }
}
const allDay = (y: number, mo: number, d: number, over: Partial<Task> = {}): Task => task({ dueMs: at(y, mo, d), isAllDay: true, ...over })

describe('toEvent', () => {
  it('skips tasks with no due date, trashed tasks, and done ones unless asked', () => {
    expect(toEvent(task())).toBeNull()
    expect(toEvent(allDay(2026, 9, 29, { deletedMs: 1 }))).toBeNull()
    expect(toEvent(allDay(2026, 9, 29, { status: 'done' }))).toBeNull()
    expect(toEvent(allDay(2026, 9, 29, { status: 'done' }), true)?.done).toBe(true)
  })

  it('makes a single-day span of an all-day task', () => {
    const e = toEvent(allDay(2026, 9, 29))!
    expect(e.kind).toBe('span')
    expect(e.startDay).toBe(at(2026, 9, 29))
    expect(e.endDay).toBe(at(2026, 9, 29))
  })

  it('draws a timed task with only a due time as a 30 minute block', () => {
    const e = toEvent(task({ dueMs: at(2026, 9, 29, 15) }))!
    expect(e.kind).toBe('timed')
    expect(e.startMs).toBe(at(2026, 9, 29, 15))
    expect(e.endMs).toBe(at(2026, 9, 29, 15, 30))
  })

  it('uses start..due as the block for a same-day timed task', () => {
    const e = toEvent(task({ startMs: at(2026, 9, 29, 9), dueMs: at(2026, 9, 29, 11) }))!
    expect(e.kind).toBe('timed')
    expect([e.startMs, e.endMs]).toEqual([at(2026, 9, 29, 9), at(2026, 9, 29, 11)])
  })

  it('spans days for a start and due on different days, timed or not', () => {
    const e = toEvent(task({ startMs: at(2026, 9, 28, 22), dueMs: at(2026, 9, 30, 8) }))!
    expect(e.kind).toBe('span')
    expect([e.startDay, e.endDay]).toEqual([at(2026, 9, 28), at(2026, 9, 30)])
    const a = toEvent(allDay(2026, 9, 30, { startMs: at(2026, 9, 27) }))!
    expect([a.startDay, a.endDay]).toEqual([at(2026, 9, 27), at(2026, 9, 30)])
  })

  it('ignores a start after the due date', () => {
    const e = toEvent(allDay(2026, 9, 29, { startMs: at(2026, 10, 5) }))!
    expect([e.startDay, e.endDay]).toEqual([at(2026, 9, 29), at(2026, 9, 29)])
  })

  it('keeps a timed task ending at midnight on the day it ended', () => {
    const e = toEvent(task({ startMs: at(2026, 9, 29, 22), dueMs: at(2026, 9, 30) }))!
    expect(e.kind).toBe('timed')
    expect(e.startDay).toBe(at(2026, 9, 29))
  })

  it('treats a zero-length timed task (start == due) as a default block', () => {
    const e = toEvent(task({ startMs: at(2026, 9, 29, 9), dueMs: at(2026, 9, 29, 9) }))!
    expect(e.endMs - e.startMs).toBe(30 * 60_000)
  })
})

describe('eventsOnDay / buildEvents', () => {
  it('lists bars before timed events, then by time', () => {
    const events = buildEvents([
      task({ title: 'late', dueMs: at(2026, 9, 29, 17) }),
      task({ title: 'early', dueMs: at(2026, 9, 29, 8) }),
      allDay(2026, 9, 29, { title: 'bar' }),
      allDay(2026, 9, 30, { title: 'other day' }),
    ])
    expect(eventsOnDay(events, at(2026, 9, 29)).map((e) => e.task.title)).toEqual(['bar', 'early', 'late'])
  })

  it('includes a multi-day event on each day it covers', () => {
    const events = buildEvents([allDay(2026, 10, 1, { startMs: at(2026, 9, 29) })])
    expect([29, 30].map((d) => eventsOnDay(events, at(2026, 9, d)).length)).toEqual([1, 1])
    expect(eventsOnDay(events, at(2026, 10, 1))).toHaveLength(1)
    expect(eventsOnDay(events, at(2026, 10, 2))).toHaveLength(0)
  })
})

describe('weekSegments', () => {
  const week = Array.from({ length: 7 }, (_, i) => addDays(at(2026, 9, 27), i)) // Sun Sep 27 - Sat Oct 3

  it('clips a bar to the row and marks the cut ends', () => {
    const events = buildEvents([allDay(2026, 10, 6, { startMs: at(2026, 9, 25) })])
    const { segments } = weekSegments(events, week)
    expect(segments).toHaveLength(1)
    expect(segments[0]).toMatchObject({ startCol: 0, endCol: 6, lane: 0, cutStart: true, cutEnd: true })
  })

  it('places a bar that starts and ends inside the row', () => {
    const events = buildEvents([allDay(2026, 9, 30, { startMs: at(2026, 9, 28) })])
    expect(weekSegments(events, week).segments[0]).toMatchObject({ startCol: 1, endCol: 3, cutStart: false, cutEnd: false })
  })

  it('skips events outside the row', () => {
    const events = buildEvents([allDay(2026, 10, 4), allDay(2026, 9, 26)])
    expect(weekSegments(events, week)).toEqual({ segments: [], lanes: 0 })
  })

  it('never overlaps bars within a lane, and reuses a freed lane', () => {
    const events = buildEvents([
      allDay(2026, 9, 29, { title: 'a', startMs: at(2026, 9, 27) }), // cols 0-2
      allDay(2026, 9, 28, { title: 'b', startMs: at(2026, 9, 28) }), // col 1
      allDay(2026, 10, 1, { title: 'c', startMs: at(2026, 9, 30) }), // cols 3-4
      allDay(2026, 9, 29, { title: 'd' }), // col 2
    ])
    const { segments, lanes } = weekSegments(events, week)
    const byTitle = Object.fromEntries(segments.map((s) => [s.event.task.title, s]))
    expect(byTitle.a!.lane).toBe(0)
    expect(byTitle.b!.lane).toBe(1)
    expect(byTitle.d!.lane).toBe(1) // b ended in col 1, so lane 1 is free again
    expect(byTitle.c!.lane).toBe(0)
    expect(lanes).toBe(2)
    for (const x of segments) for (const z of segments) if (x !== z && x.lane === z.lane) expect(x.endCol < z.startCol || z.endCol < x.startCol).toBe(true)
  })

  it('counts what falls past the lane limit per column', () => {
    const events = buildEvents([1, 2, 3, 4, 5].map((i) => allDay(2026, 9, 29, { title: `t${i}` })))
    const { segments } = weekSegments(events, week)
    expect(hiddenPerColumn(segments, 7, 3)).toEqual([0, 0, 2, 0, 0, 0, 0])
  })

  it('handles a week containing the spring-forward change (Mar 8 2026)', () => {
    const w = Array.from({ length: 7 }, (_, i) => addDays(at(2026, 3, 8), i))
    const events = buildEvents([allDay(2026, 3, 14, { startMs: at(2026, 3, 8) })])
    expect(weekSegments(events, w).segments[0]).toMatchObject({ startCol: 0, endCol: 6 })
    const e2 = buildEvents([allDay(2026, 3, 10)])
    expect(weekSegments(e2, w).segments[0]).toMatchObject({ startCol: 2, endCol: 2 })
  })

  it('handles a week containing the fall-back change (Nov 1 2026)', () => {
    const w = Array.from({ length: 7 }, (_, i) => addDays(at(2026, 11, 1), i))
    expect(w.map((d) => new Date(d).getDate())).toEqual([1, 2, 3, 4, 5, 6, 7])
    const events = buildEvents([allDay(2026, 11, 2, { startMs: at(2026, 11, 1) })])
    expect(weekSegments(events, w).segments[0]).toMatchObject({ startCol: 0, endCol: 1 })
  })
})

describe('layoutTimed', () => {
  const day = at(2026, 9, 29)
  const ev = (h1: number, m1: number, h2: number, m2: number, title = 'x') => task({ title, startMs: at(2026, 9, 29, h1, m1), dueMs: at(2026, 9, 29, h2, m2) })

  it('positions a block as a percentage of the day', () => {
    const [b] = layoutTimed(buildEvents([ev(6, 0, 12, 0)]), day)
    expect(b).toMatchObject({ top: 25, height: 25, col: 0, cols: 1 })
  })

  it('splits overlapping events into columns and lets later ones reuse space', () => {
    const blocks = layoutTimed(buildEvents([ev(9, 0, 11, 0, 'a'), ev(10, 0, 12, 0, 'b'), ev(11, 0, 12, 0, 'c'), ev(14, 0, 15, 0, 'd')]), day)
    const by = Object.fromEntries(blocks.map((b) => [b.event.task.title, b]))
    expect([by.a!.col, by.b!.col, by.c!.col]).toEqual([0, 1, 0]) // c starts as a ends
    expect([by.a!.cols, by.b!.cols, by.c!.cols]).toEqual([2, 2, 2])
    expect([by.d!.col, by.d!.cols]).toEqual([0, 1])
  })

  it('does not treat back-to-back events as overlapping', () => {
    const blocks = layoutTimed(buildEvents([ev(9, 0, 10, 0), ev(10, 0, 11, 0)]), day)
    expect(blocks.every((b) => b.cols === 1)).toBe(true)
  })

  it('gives a due-only task a 30 minute block', () => {
    const [b] = layoutTimed(buildEvents([task({ dueMs: at(2026, 9, 29, 15) })]), day)
    expect(b!.endMin - b!.startMin).toBe(30)
  })

  it('clips a block that would run past midnight', () => {
    const [b] = layoutTimed(buildEvents([task({ dueMs: at(2026, 9, 29, 23, 45) })]), day)
    expect(b!.endMin).toBe(1440)
    expect(b!.top + b!.height).toBeCloseTo(100)
    const [c] = layoutTimed(buildEvents([task({ startMs: at(2026, 9, 29, 22), dueMs: at(2026, 9, 30) })]), day)
    expect(c!.endMin).toBe(1440)
  })

  it('only includes timed events that start that day', () => {
    expect(layoutTimed(buildEvents([allDay(2026, 9, 29), task({ dueMs: at(2026, 9, 30, 9) })]), day)).toEqual([])
  })

  it('uses wall-clock time on a DST day', () => {
    const springDay = at(2026, 3, 8)
    const [b] = layoutTimed(buildEvents([task({ dueMs: at(2026, 3, 8, 15) })]), springDay)
    expect(b!.startMin).toBe(15 * 60)
    expect(minutesOfDay(at(2026, 11, 1, 3, 15))).toBe(195)
  })
})

describe('slotTime', () => {
  it('snaps down and stays inside the day', () => {
    expect(slotTime(at(2026, 9, 29), 9 * 60 + 40)).toBe(at(2026, 9, 29, 9, 30))
    expect(slotTime(at(2026, 9, 29), -5)).toBe(at(2026, 9, 29, 0, 0))
    expect(slotTime(at(2026, 9, 29), 5000)).toBe(at(2026, 9, 29, 23, 45))
  })
})

describe('rescheduling', () => {
  it('moves by days keeping time of day, start and due', () => {
    const t = task({ startMs: at(2026, 3, 7, 9), dueMs: at(2026, 3, 7, 10, 30) })
    expect(moveByDays(t, 2)).toEqual({ startMs: at(2026, 3, 9, 9), dueMs: at(2026, 3, 9, 10, 30), isAllDay: false }) // across the DST change
    expect(moveByDays(allDay(2026, 9, 29), -1)).toEqual({ dueMs: at(2026, 9, 28), isAllDay: true })
  })

  it('moves a timed task to a slot keeping its length', () => {
    const t = task({ startMs: at(2026, 9, 29, 9), dueMs: at(2026, 9, 29, 10, 30) })
    expect(moveToSlot(t, at(2026, 10, 1, 14))).toEqual({ startMs: at(2026, 10, 1, 14), dueMs: at(2026, 10, 1, 15, 30), isAllDay: false })
    expect(moveToSlot(task({ dueMs: at(2026, 9, 29, 9) }), at(2026, 9, 30, 13))).toEqual({ dueMs: at(2026, 9, 30, 13), isAllDay: false })
  })

  it('makes an all-day task timed when dropped on a slot', () => {
    expect(moveToSlot(allDay(2026, 9, 29), at(2026, 9, 30, 8))).toEqual({ dueMs: at(2026, 9, 30, 8), isAllDay: false })
    expect(moveToSlot(allDay(2026, 9, 29, { startMs: at(2026, 9, 27) }), at(2026, 9, 30, 8))).toEqual({ dueMs: at(2026, 9, 30, 8), startMs: null, isAllDay: false })
  })

  it('makes a timed task all-day when dropped on the strip, and moves spans as a unit', () => {
    expect(moveToAllDay(task({ dueMs: at(2026, 9, 29, 15) }), at(2026, 10, 2))).toEqual({ dueMs: at(2026, 10, 2), isAllDay: true })
    expect(moveToAllDay(task({ startMs: at(2026, 9, 29, 9), dueMs: at(2026, 9, 29, 10) }), at(2026, 10, 2))).toEqual({ dueMs: at(2026, 10, 2), startMs: null, isAllDay: true })
    const span = allDay(2026, 10, 1, { startMs: at(2026, 9, 29) })
    expect(moveToAllDay(span, at(2026, 10, 5))).toEqual({ startMs: at(2026, 10, 5), dueMs: at(2026, 10, 7), isAllDay: true })
  })
})

describe('visibleRange and stepping', () => {
  it.each([[0, 'Sun'], [1, 'Mon'], [6, 'Sat']] as const)('month grid starts on the week start (%s, %s)', (ws, _name) => {
    void _name
    const r = visibleRange('m', at(2026, 9, 29), ws)
    expect(r.days).toHaveLength(42)
    expect(new Date(r.days[0]!).getDay()).toBe(ws)
    expect(r.days).toEqual(monthGrid(at(2026, 9, 29), ws))
    expect(r.end).toBe(addDays(r.days[41]!, 1))
    expect(r.days.some((d) => d === at(2026, 9, 1))).toBe(true)
    expect(r.days.some((d) => d === at(2026, 9, 30))).toBe(true)
  })

  it('week is seven days from the week start, day is one, agenda is 30', () => {
    expect(visibleRange('w', at(2026, 9, 29), 0).days.map((d) => new Date(d).getDate())).toEqual([27, 28, 29, 30, 1, 2, 3])
    expect(visibleRange('w', at(2026, 9, 29), 1).days.map((d) => new Date(d).getDate())).toEqual([28, 29, 30, 1, 2, 3, 4])
    expect(visibleRange('w', at(2026, 9, 29), 6).days[0]).toBe(at(2026, 9, 26))
    expect(visibleRange('d', at(2026, 9, 29, 15), 1).days).toEqual([at(2026, 9, 29)])
    const a = visibleRange('a', at(2026, 9, 29, 15), 1)
    expect(a.days).toHaveLength(30)
    expect(a.start).toBe(at(2026, 9, 29))
  })

  it('multi-day modes start on the anchor day and page by their length', () => {
    const three = visibleRange('3', at(2026, 9, 29, 15), 1)
    expect(three.days.map((d) => new Date(d).getDate())).toEqual([29, 30, 1])
    expect(three.end).toBe(at(2026, 10, 2))
    expect(visibleRange('t', at(2026, 9, 29), 0).days).toHaveLength(10)
    expect(stepAnchor('3', at(2026, 9, 29), 1)).toBe(at(2026, 10, 2))
    expect(stepAnchor('t', at(2026, 9, 29), -1)).toBe(at(2026, 9, 19))
    expect(parseMode('3')).toBe('3')
    expect(parseMode('t')).toBe('t')
  })

  it('a week across the fall-back change still has seven calendar days', () => {
    const r = visibleRange('w', at(2026, 11, 1, 12), 0)
    expect(r.days.map((d) => new Date(d).getDate())).toEqual([1, 2, 3, 4, 5, 6, 7])
    expect(r.days.every((d) => d === startOfDay(d))).toBe(true)
  })

  it('steps by month, week, day and agenda page, across year boundaries', () => {
    expect(stepAnchor('m', at(2026, 12, 15), 1)).toBe(at(2027, 1, 15))
    expect(stepAnchor('m', at(2026, 1, 31), 1)).toBe(at(2026, 2, 28))
    expect(stepAnchor('m', at(2026, 1, 15), -1)).toBe(at(2025, 12, 15))
    expect(stepAnchor('w', at(2026, 12, 30), 1)).toBe(at(2027, 1, 6))
    expect(stepAnchor('d', at(2026, 3, 8, 12), 1)).toBe(at(2026, 3, 9, 12))
    expect(stepAnchor('a', at(2026, 9, 29), 1)).toBe(at(2026, 10, 29))
    expect(stepAnchor('a', at(2026, 9, 29), -1)).toBe(at(2026, 8, 30))
  })

  it('parses modes, falling back to month', () => {
    expect(['m', 'w', 'd', 'a', 'x', undefined].map((m) => parseMode(m))).toEqual(['m', 'w', 'd', 'a', 'm', 'm'])
  })

  it('splits days into week rows', () => {
    expect(chunkWeeks(visibleRange('m', at(2026, 9, 29), 1).days).map((w) => w.length)).toEqual([7, 7, 7, 7, 7, 7])
  })
})

describe('titles and labels', () => {
  const title = (mode: 'm' | 'w' | 'd' | '3' | 't' | 'a', anchor: number, ws: 0 | 1 | 6 = 0): string => rangeTitle(mode, anchor, visibleRange(mode, anchor, ws))
  it('titles each mode', () => {
    expect(title('m', at(2026, 9, 29))).toBe('September 2026')
    expect(title('w', at(2026, 9, 29))).toBe('Sep 27 – Oct 3, 2026')
    expect(title('w', at(2026, 9, 10))).toBe('Sep 6 – 12, 2026')
    expect(title('d', at(2026, 9, 29))).toBe('Tuesday, Sep 29, 2026')
    expect(title('w', at(2026, 12, 30))).toBe('Dec 27, 2026 – Jan 2, 2027')
    expect(title('a', at(2026, 9, 29))).toBe('Sep 29 – Oct 28, 2026')
  })

  it('labels a cell', () => {
    expect(cellLabel(at(2026, 9, 29), 3)).toBe('Tuesday, September 29, 2026, 3 tasks')
    expect(cellLabel(at(2026, 9, 29), 1)).toBe('Tuesday, September 29, 2026, 1 task')
  })

  it('says when an event happens', () => {
    const e = toEvent(task({ startMs: at(2026, 9, 29, 9), dueMs: at(2026, 9, 29, 10, 30) }))!
    expect(eventWhen(e, true)).toBe('Tue, Sep 29, 9:00 AM – 10:30 AM')
    expect(eventWhen(toEvent(allDay(2026, 9, 29))!, false)).toBe('Tue, Sep 29')
    expect(eventWhen(toEvent(allDay(2026, 9, 30, { startMs: at(2026, 9, 28) }))!, false)).toBe('Mon, Sep 28 – Wed, Sep 30')
  })
})

describe('agendaGroups', () => {
  it('groups by day, skips empty days and ignores events outside the range', () => {
    const range = visibleRange('a', at(2026, 9, 29), 1)
    const events = buildEvents([allDay(2026, 9, 29), task({ dueMs: at(2026, 10, 2, 9) }), allDay(2026, 10, 2), allDay(2026, 9, 28), allDay(2026, 12, 1)])
    const groups = agendaGroups(events, range)
    expect(groups.map((g) => [new Date(g.day).getDate(), g.events.length])).toEqual([[29, 1], [2, 2]])
  })
})

describe('repeating tasks', () => {
  const daily = (over: Partial<Task> = {}) => allDay(2026, 9, 29, { repeatFlag: 'RRULE:FREQ=DAILY', ...over })

  it('shows later occurrences up to the end of the range, marked as projected', () => {
    const events = buildEvents([daily()], false, at(2026, 10, 2))
    expect(events.map((e) => [e.startDay, e.projected])).toEqual([[at(2026, 9, 29), false], [at(2026, 9, 30), true], [at(2026, 10, 1), true]])
  })
  it('keeps the time of day and skips excluded days', () => {
    const t = task({ dueMs: at(2026, 9, 29, 9, 30), repeatFlag: 'RRULE:FREQ=DAILY', exDates: [at(2026, 9, 30)] })
    const events = buildEvents([t], false, at(2026, 10, 2))
    expect(events.map((e) => e.startMs)).toEqual([at(2026, 9, 29, 9, 30), at(2026, 10, 1, 9, 30)])
  })
  it('draws only the task itself when no end is given, and nothing extra for done or non-repeating tasks', () => {
    expect(buildEvents([daily()])).toHaveLength(1)
    expect(buildEvents([daily({ status: 'done' })], true, at(2026, 10, 9))).toHaveLength(1)
    expect(buildEvents([allDay(2026, 9, 29)], false, at(2026, 10, 9))).toHaveLength(1)
  })
})

describe('overdueEvents', () => {
  it('lists open, non-projected events from before today, oldest first', () => {
    const today = at(2026, 9, 29)
    const events = buildEvents([allDay(2026, 9, 27, { title: 'b' }), allDay(2026, 9, 25, { title: 'a' }), allDay(2026, 9, 29), allDay(2026, 9, 20, { status: 'done' })], true, at(2026, 10, 5))
    expect(overdueEvents(events, today).map((e) => e.task.title)).toEqual(['a', 'b'])
  })
})

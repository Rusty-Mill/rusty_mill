import { CALENDAR_MODES } from '@/app/paths';
import { addDays, addMonths, atTime, diffDays, formatTime, monthGrid, startOfDay, startOfWeek, MINUTE } from '@/lib/date';
import { laterOccurrences } from '@/lib/recurrence';
export const DEFAULT_COLOR = '#4772fa';
/** A timed task with no end is drawn as a block this long. */
export const DEFAULT_DURATION_MIN = 30;
export const AGENDA_DAYS = 30;
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const short = (s) => s.slice(0, 3);
/** Days shown by the multi-day modes, which start on the anchor day and page by that many days. */
export const MULTI_DAYS = { '3': 3, t: 10 };
export const parseMode = (s) => (CALENDAR_MODES.includes(s ?? '') ? s : 'm');
/** The event a task draws as, or `null` if it has no place on a calendar. */
export function toEvent(task, showDone = false) {
    if (task.deletedMs !== null || task.dueMs === null)
        return null;
    const done = task.status !== 'open';
    if (done && !showDone)
        return null;
    const due = task.dueMs;
    // A start after the due date is bad data, not a reversed span.
    const start = task.startMs !== null && task.startMs <= due ? task.startMs : due;
    const startDay = startOfDay(start);
    let endDay = startOfDay(due);
    // A timed task ending exactly at midnight belongs to the day it ended, not the next one.
    if (!task.isAllDay && due === endDay && due > start)
        endDay = startOfDay(due - 1);
    if (task.isAllDay || startDay !== endDay)
        return { task, kind: 'span', startDay, endDay: Math.max(startDay, endDay), startMs: start, endMs: due, done, projected: false };
    const end = due > start ? due : start + DEFAULT_DURATION_MIN * MINUTE;
    return { task, kind: 'timed', startDay, endDay, startMs: start, endMs: end, done, projected: false };
}
/** Events for `tasks`; with `until`, repeating tasks also show their later occurrences before it. */
export function buildEvents(tasks, showDone = false, until) {
    const out = [];
    for (const t of tasks) {
        const e = toEvent(t, showDone);
        if (!e)
            continue;
        out.push(e);
        if (until === undefined || t.status !== 'open' || !t.repeatFlag)
            continue;
        for (const o of laterOccurrences(t, until, t.exDates)) {
            const later = toEvent({ ...t, dueMs: o.dueMs, startMs: o.startMs });
            if (later)
                out.push({ ...later, projected: true });
        }
    }
    return out;
}
/** Bars first (longest first), then timed by time; ties by title so the order is stable. */
export function compareEvents(a, b) {
    if (a.kind !== b.kind)
        return a.kind === 'span' ? -1 : 1;
    if (a.kind === 'span' && a.startDay !== b.startDay)
        return a.startDay - b.startDay;
    const lenA = a.endDay - a.startDay;
    const lenB = b.endDay - b.startDay;
    if (a.kind === 'span' && lenA !== lenB)
        return lenB - lenA;
    return a.startMs - b.startMs || a.task.title.localeCompare(b.task.title) || a.task.id.localeCompare(b.task.id);
}
export const coversDay = (e, day) => e.startDay <= day && day <= e.endDay;
/** Events covering `day`, in display order. */
export function eventsOnDay(events, day) {
    return events.filter((e) => coversDay(e, day)).sort(compareEvents);
}
/**
 * Bars for one row of days, each in the first lane that is free for its whole
 * width, so bars never overlap and a multi-day bar keeps its lane across days.
 */
export function weekSegments(events, days) {
    const first = days[0];
    const last = days[days.length - 1];
    if (first === undefined || last === undefined)
        return { segments: [], lanes: 0 };
    const inRow = events.filter((e) => e.endDay >= first && e.startDay <= last).sort(compareEvents);
    const laneEnds = []; // last occupied column per lane
    const segments = [];
    for (const event of inRow) {
        const startCol = Math.max(0, diffDays(first, event.startDay));
        const endCol = Math.min(days.length - 1, diffDays(first, event.endDay));
        let lane = laneEnds.findIndex((end) => end < startCol);
        if (lane < 0)
            lane = laneEnds.length;
        laneEnds[lane] = endCol;
        segments.push({ event, startCol, endCol, lane, cutStart: event.startDay < first, cutEnd: event.endDay > last });
    }
    return { segments, lanes: laneEnds.length };
}
/** Per column, how many bars sit in a lane at or beyond `maxLanes` (the "+N more"). */
export function hiddenPerColumn(segments, columns, maxLanes) {
    const out = new Array(columns).fill(0);
    for (const s of segments) {
        if (s.lane < maxLanes)
            continue;
        for (let c = s.startCol; c <= s.endCol; c++)
            out[c] = (out[c] ?? 0) + 1;
    }
    return out;
}
export const chunkWeeks = (days) => Array.from({ length: Math.ceil(days.length / 7) }, (_, i) => days.slice(i * 7, i * 7 + 7));
export const MINUTES_PER_DAY = 1440;
/** Wall-clock minutes since local midnight. */
export function minutesOfDay(ms) {
    const d = new Date(ms);
    return d.getHours() * 60 + d.getMinutes();
}
/** Blocks for one day: overlapping events share the width in columns. */
export function layoutTimed(events, day) {
    const nextDay = addDays(day, 1);
    const items = events
        .filter((e) => e.kind === 'timed' && e.startDay === day)
        .map((event) => {
        const startMin = minutesOfDay(event.startMs);
        // Clipped at midnight: a block never spills into the next column.
        const endMin = event.endMs >= nextDay ? MINUTES_PER_DAY : Math.max(minutesOfDay(event.endMs), startMin + 1);
        return { event, startMin, endMin };
    })
        .sort((a, b) => a.startMin - b.startMin || b.endMin - a.endMin || a.event.task.id.localeCompare(b.event.task.id));
    const out = [];
    let cluster = [];
    let colEnds = [];
    let clusterEnd = -1;
    const flush = () => {
        for (const c of cluster) {
            out.push({ ...c, top: (c.startMin / MINUTES_PER_DAY) * 100, height: ((c.endMin - c.startMin) / MINUTES_PER_DAY) * 100, cols: colEnds.length });
        }
        cluster = [];
        colEnds = [];
    };
    for (const it of items) {
        if (cluster.length > 0 && it.startMin >= clusterEnd)
            flush();
        let col = colEnds.findIndex((end) => end <= it.startMin);
        if (col < 0)
            col = colEnds.length;
        colEnds[col] = it.endMin;
        clusterEnd = cluster.length === 0 ? it.endMin : Math.max(clusterEnd, it.endMin);
        cluster.push({ ...it, col });
    }
    flush();
    return out;
}
/** The time `minutes` past midnight on `day`, snapped down to a multiple of `step`. */
export function slotTime(day, minutes, step = 15) {
    const m = Math.max(0, Math.min(MINUTES_PER_DAY - step, Math.floor(minutes / step) * step));
    return atTime(day, Math.floor(m / 60), m % 60);
}
/** Move by whole days, keeping the time of day and the length. */
export function moveByDays(task, days) {
    const out = { dueMs: addDays(task.dueMs ?? 0, days), isAllDay: task.isAllDay };
    if (task.startMs !== null)
        out.startMs = addDays(task.startMs, days);
    return out;
}
/** Drop onto a time slot: becomes timed, its start moves to `slotMs`, its length is kept. */
export function moveToSlot(task, slotMs) {
    const due = task.dueMs ?? slotMs;
    const start = task.startMs !== null && task.startMs <= due ? task.startMs : due;
    if (task.isAllDay)
        return task.startMs === null ? { dueMs: slotMs, isAllDay: false } : { dueMs: slotMs, startMs: null, isAllDay: false };
    const out = { dueMs: due + (slotMs - start), isAllDay: false };
    if (task.startMs !== null)
        out.startMs = slotMs;
    return out;
}
/** Drop onto a day in the all-day strip: the start lands on `day`, a multi-day span keeps its length. */
export function moveToAllDay(task, day) {
    const due = task.dueMs ?? day;
    const start = task.startMs !== null && task.startMs <= due ? task.startMs : due;
    const delta = diffDays(start, day);
    const out = { dueMs: startOfDay(addDays(due, delta)), isAllDay: true };
    if (task.startMs !== null)
        out.startMs = startOfDay(start) === startOfDay(due) ? null : startOfDay(addDays(start, delta));
    return out;
}
export function visibleRange(mode, anchor, weekStart) {
    let days;
    if (mode === 'm')
        days = monthGrid(anchor, weekStart);
    else if (mode === 'w') {
        const s = startOfWeek(anchor, weekStart);
        days = Array.from({ length: 7 }, (_, i) => addDays(s, i));
    }
    else if (mode === 'd')
        days = [startOfDay(anchor)];
    else if (mode === '3' || mode === 't')
        days = Array.from({ length: MULTI_DAYS[mode] }, (_, i) => addDays(startOfDay(anchor), i));
    else
        days = Array.from({ length: AGENDA_DAYS }, (_, i) => addDays(startOfDay(anchor), i));
    const first = days[0] ?? startOfDay(anchor);
    const last = days[days.length - 1] ?? first;
    return { start: first, end: addDays(last, 1), days };
}
/** The anchor after moving one page (`dir` is 1 or -1) in `mode`. */
export function stepAnchor(mode, anchor, dir) {
    if (mode === 'm')
        return addMonths(anchor, dir);
    if (mode === 'w')
        return addDays(anchor, 7 * dir);
    if (mode === 'd')
        return addDays(anchor, dir);
    if (mode === '3' || mode === 't')
        return addDays(anchor, MULTI_DAYS[mode] * dir);
    return addDays(anchor, AGENDA_DAYS * dir);
}
const md = (ms) => `${short(MONTHS[new Date(ms).getMonth()] ?? '')} ${new Date(ms).getDate()}`;
const y = (ms) => new Date(ms).getFullYear();
/** `September 2026`, `Sep 27 – Oct 3, 2026`, `Tuesday, Sep 29, 2026`. */
export function rangeTitle(mode, anchor, range) {
    const a = new Date(anchor);
    if (mode === 'm')
        return `${MONTHS[a.getMonth()]} ${a.getFullYear()}`;
    if (mode === 'd')
        return `${WEEKDAYS[a.getDay()]}, ${md(anchor)}, ${a.getFullYear()}`;
    const first = range.days[0] ?? anchor;
    const last = range.days[range.days.length - 1] ?? first;
    if (y(first) !== y(last))
        return `${md(first)}, ${y(first)} – ${md(last)}, ${y(last)}`;
    return `${md(first)} – ${new Date(first).getMonth() === new Date(last).getMonth() && mode !== 'a' ? new Date(last).getDate() : md(last)}, ${y(last)}`;
}
// ---- labels -----------------------------------------------------------
/** `Tuesday, September 29, 2026` */
export function longDate(ms) {
    const d = new Date(ms);
    return `${WEEKDAYS[d.getDay()]}, ${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`;
}
export const cellLabel = (day, count) => `${longDate(day)}, ${count} ${count === 1 ? 'task' : 'tasks'}`;
/** `Tue, Sep 29` */
export function shortDate(ms) {
    const d = new Date(ms);
    return `${short(WEEKDAYS[d.getDay()] ?? '')}, ${md(ms)}`;
}
export const weekdayShort = (i) => short(WEEKDAYS[i] ?? '');
/** When an event happens, for a popover: `Tue, Sep 29, 3:00 PM`, `Sep 28 – Sep 30`. */
export function eventWhen(e, hour12) {
    const t = e.task;
    if (e.kind === 'timed') {
        const s = formatTime(e.startMs, hour12);
        return t.startMs !== null && e.endMs > e.startMs ? `${shortDate(e.startDay)}, ${s} – ${formatTime(e.endMs, hour12)}` : `${shortDate(e.startDay)}, ${s}`;
    }
    if (e.startDay === e.endDay)
        return shortDate(e.startDay);
    const at = (ms, day) => (t.isAllDay ? shortDate(day) : `${shortDate(day)}, ${formatTime(ms, hour12)}`);
    return `${at(e.startMs, e.startDay)} – ${at(e.endMs, e.endDay)}`;
}
/** The short time prefix of a bar in the month grid, or `''` for all-day and multi-day. */
export const barTime = (e, hour12) => (e.kind === 'timed' ? formatTime(e.startMs, hour12) : '');
/** Days in `range` with at least one event, each with its events. */
export function agendaGroups(events, range) {
    return range.days.map((day) => ({ day, events: eventsOnDay(events, day) })).filter((g) => g.events.length > 0);
}
/** Open tasks whose day has passed, oldest first: what the agenda lists above today. */
export function overdueEvents(events, today) {
    return events.filter((e) => !e.done && !e.projected && e.endDay < today).sort((a, b) => a.endDay - b.endDay || a.task.title.localeCompare(b.task.title));
}

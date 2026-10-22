import { describe, expect, it } from 'vitest';
import { MAX_EX_DATES, parseIcs, storedExDates } from './ics';
const wrap = (...items) => ['BEGIN:VCALENDAR', 'VERSION:2.0', ...items, 'END:VCALENDAR'].join('\r\n');
const event = (...lines) => ['BEGIN:VEVENT', ...lines, 'END:VEVENT'].join('\r\n');
describe('parseIcs', () => {
    it('reads a UTC event as a start and a due time', () => {
        const { tasks } = parseIcs(wrap(event('SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'DTEND:20261005T093000Z')));
        expect(tasks).toEqual([
            { uid: `Standup|${Date.UTC(2026, 9, 5, 9)}|${Date.UTC(2026, 9, 5, 9, 30)}`, title: 'Standup', notes: '', startMs: Date.UTC(2026, 9, 5, 9), dueMs: Date.UTC(2026, 9, 5, 9, 30), isAllDay: false, timeZone: '', repeatFlag: '', recurrenceId: null, exDates: [] },
        ]);
    });
    it('treats an all-day DTEND as exclusive and a one-day event as a due date only', () => {
        const { tasks } = parseIcs(wrap(event('SUMMARY:Trip', 'DTSTART;VALUE=DATE:20261010', 'DTEND;VALUE=DATE:20261013'), event('SUMMARY:Holiday', 'DTSTART;VALUE=DATE:20261105', 'DTEND;VALUE=DATE:20261106')));
        expect(tasks[0]).toMatchObject({ isAllDay: true, startMs: new Date(2026, 9, 10).getTime(), dueMs: new Date(2026, 9, 12).getTime() });
        expect(tasks[1]).toMatchObject({ isAllDay: true, startMs: null, dueMs: new Date(2026, 10, 5).getTime() });
    });
    it('converts a TZID wall time to the right instant, DST included', () => {
        const { tasks } = parseIcs(wrap(event('SUMMARY:NY summer', 'DTSTART;TZID=America/New_York:20260705T120000'), event('SUMMARY:NY winter', 'DTSTART;TZID=America/New_York:20261205T120000')));
        expect(tasks[0]).toMatchObject({ dueMs: Date.UTC(2026, 6, 5, 16), timeZone: 'America/New_York' });
        expect(tasks[1]).toMatchObject({ dueMs: Date.UTC(2026, 11, 5, 17) });
    });
    it('keeps the feed UID, which stays the same when the event is edited', () => {
        const one = parseIcs(wrap(event('UID:abc@x', 'SUMMARY:Old', 'DTSTART;VALUE=DATE:20261010'))).tasks[0];
        const two = parseIcs(wrap(event('UID:abc@x', 'SUMMARY:New', 'DTSTART;VALUE=DATE:20261011'))).tasks[0];
        expect([one.uid, two.uid]).toEqual(['abc@x', 'abc@x']);
    });
    it('unfolds long lines and unescapes text', () => {
        const { tasks } = parseIcs(wrap(event('SUMMARY:Plan\\, review', 'DESCRIPTION:line one\\nline\r\n  two\; end', 'DTSTART;VALUE=DATE:20261010')));
        expect(tasks[0]).toMatchObject({ title: 'Plan, review', notes: 'line one\nline two; end' });
    });
    it('keeps repeat rules the app understands and counts the ones it does not', () => {
        const { tasks, droppedRepeats } = parseIcs(wrap(event('SUMMARY:Weekly', 'DTSTART;VALUE=DATE:20261010', 'RRULE:FREQ=WEEKLY;BYDAY=MO'), event('SUMMARY:Odd', 'DTSTART;VALUE=DATE:20261010', 'RRULE:FREQ=SECONDLY')));
        expect(tasks[0].repeatFlag).toBe('RRULE:FREQ=WEEKLY;BYDAY=MO');
        expect(tasks[1].repeatFlag).toBe('');
        expect(droppedRepeats).toBe(1);
    });
    it('imports a todo by its DUE date and skips cancelled, completed, untitled and undated items', () => {
        const todo = ['BEGIN:VTODO', 'SUMMARY:Pay rent', 'DUE:20261101T080000Z', 'BEGIN:VALARM', 'TRIGGER:-PT15M', 'END:VALARM', 'END:VTODO'].join('\r\n');
        const { tasks, skipped } = parseIcs(wrap(todo, event('SUMMARY:X', 'STATUS:CANCELLED', 'DTSTART:20261005T090000Z'), event('SUMMARY:Y', 'STATUS:COMPLETED', 'DTSTART:20261005T090000Z'), event('DTSTART:20261005T090000Z'), event('SUMMARY:Undated')));
        expect(tasks.map((t) => t.title)).toEqual(['Pay rent']);
        expect(tasks[0]).toMatchObject({ startMs: null, dueMs: Date.UTC(2026, 10, 1, 8) });
        expect(skipped).toBe(4);
    });
    it('returns nothing for text that is not a calendar', () => {
        expect(parseIcs('hello')).toEqual({ tasks: [], skipped: 0, droppedRepeats: 0, droppedExceptions: 0 });
    });
    it('puts exceptions of an event with an end on its due timeline: EXDATE and moved instances alike', () => {
        const { tasks, droppedExceptions } = parseIcs(wrap(event('UID:w', 'SUMMARY:Sync', 'DTSTART:20261005T090000Z', 'DTEND:20261005T100000Z', 'RRULE:FREQ=WEEKLY', 'EXDATE:20261019T090000Z'), event('UID:w', 'RECURRENCE-ID:20261012T090000Z', 'SUMMARY:Sync', 'DTSTART:20261013T090000Z', 'DTEND:20261013T100000Z')));
        expect(droppedExceptions).toBe(0);
        expect(tasks[0]).toMatchObject({ recurrenceId: null, exDates: [Date.UTC(2026, 9, 19, 10), Date.UTC(2026, 9, 12, 10)] });
        expect(tasks[1]).toMatchObject({ recurrenceId: Date.UTC(2026, 9, 12, 9), exDates: [] });
    });
});
describe('storedExDates', () => {
    it('dedupes and sorts', () => {
        expect(storedExDates([30, 10, 20, 10])).toEqual([10, 20, 30]);
        expect(storedExDates([])).toEqual([]);
    });
    it('keeps the latest MAX_EX_DATES past the cap', () => {
        const many = Array.from({ length: MAX_EX_DATES + 500 }, (_, i) => MAX_EX_DATES + 500 - i); // descending, 1..1500
        const kept = storedExDates(many);
        expect(kept).toHaveLength(MAX_EX_DATES);
        expect(kept[0]).toBe(501);
        expect(kept.at(-1)).toBe(MAX_EX_DATES + 500);
    });
});

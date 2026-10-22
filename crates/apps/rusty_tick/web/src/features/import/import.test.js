import { describe, expect, it } from 'vitest';
import { MemoryAdapter } from '@/api/memory';
import { recordMerge } from './aliases';
import { applyImport, readImport } from '.';
import { legacyKeys, parseWhen } from './model';
import { buildEvents } from '@/features/calendar/layout';
import { dayKey, startOfDay } from '@/lib/date';
import { derivedId } from '@/lib/id';
const csv = (...rows) => rows.map((r) => r.map((v) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)).join(',')).join('\r\n');
const TICK_HEADER = ['Folder Name', 'List Name', 'Title', 'Kind', 'Tags', 'Content', 'Is Check list', 'Start Date', 'Due Date', 'Reminder', 'Repeat', 'Priority', 'Status', 'Created Time', 'Completed Time', 'Order', 'Timezone', 'Is All Day', 'Is Floating', 'Column Name', 'Column Order', 'View Mode', 'taskId', 'parentId'];
const tick = (o) => TICK_HEADER.map((h) => o[h] ?? '');
const ticktick = (...rows) => ['"Date: 2026-10-21+0000"', '"Version: 7.1"', '"Status: \n0 Normal\n1 Completed\n2 Archived"', csv(TICK_HEADER, ...rows.map(tick))].join('\n');
const NOW = new Date(2026, 9, 21, 12).getTime();
describe('parseWhen', () => {
    it('reads a date alone as all day at local midnight', () => {
        expect(parseWhen('2026-10-05')).toEqual({ ms: new Date(2026, 9, 5).getTime(), allDay: true });
    });
    it('reads offsets with and without a colon, Z, fractions, and no zone as local', () => {
        expect(parseWhen('2026-10-05T14:00:00+0200')?.ms).toBe(Date.UTC(2026, 9, 5, 12));
        expect(parseWhen('2026-10-05T14:00:00-05:30')?.ms).toBe(Date.UTC(2026, 9, 5, 19, 30));
        expect(parseWhen('2026-10-05T09:00:00.000Z')?.ms).toBe(Date.UTC(2026, 9, 5, 9));
        expect(parseWhen('2026-10-05 09:30')?.ms).toBe(new Date(2026, 9, 5, 9, 30).getTime());
    });
    it('puts an all-day instant on its calendar day in the given zone', () => {
        // Midnight in Shanghai is 16:00 UTC the day before.
        expect(parseWhen('2026-10-04T16:00:00+0000', true, 'Asia/Shanghai')).toEqual({ ms: new Date(2026, 9, 5).getTime(), allDay: true });
    });
    it('refuses what it cannot read', () => {
        expect(parseWhen('next tuesday')).toBeNull();
        expect(parseWhen('')).toBeNull();
    });
    it('refuses out-of-range fields instead of rolling them over into a different deadline', () => {
        for (const bad of ['2026-02-30', '2026-02-29', '2026-13-01', '2026-00-10', '2026-04-31', '2026-10-05T25:00', '2026-10-05T24:00', '2026-10-05T10:60', '2026-10-05T10:00:60', '2026-10-05T10:00+15:00', '2026-10-05T10:00+0560']) {
            expect(parseWhen(bad), bad).toBeNull();
        }
    });
    it('accepts the edges of the valid ranges, leap days included', () => {
        expect(parseWhen('2028-02-29')?.ms).toBe(new Date(2028, 1, 29).getTime());
        expect(parseWhen('2000-02-29')?.ms).toBe(new Date(2000, 1, 29).getTime());
        expect(parseWhen('1900-02-29')).toBeNull();
        expect(parseWhen('2026-12-31T23:59:59Z')?.ms).toBe(Date.UTC(2026, 11, 31, 23, 59, 59));
        expect(parseWhen('2026-10-05T00:00+14:00')?.ms).toBe(Date.UTC(2026, 9, 4, 10));
    });
});
describe('readImport: TickTick backup', () => {
    it('skips the preamble and reads lists, dates, priority, status, tags, repeat and reminders', () => {
        const r = readImport(ticktick({ 'List Name': 'Work', Title: 'Ship it', Tags: 'urgent, q4', Content: 'details', 'Due Date': '2026-10-05T14:00:00+0000', Priority: '5', Status: '0', Repeat: 'RRULE:FREQ=WEEKLY;INTERVAL=1', Reminder: 'TRIGGER:PT0S,-PT5M', Timezone: 'Europe/London', taskId: 'a1' }, { 'List Name': 'Inbox', Title: 'Done one', Status: '2', taskId: 'a2' }, { 'List Name': 'Inbox', Title: 'Not doing', Status: '-1', taskId: 'a3' }, { 'List Name': 'Inbox', Title: '', taskId: 'a4' }), 'TickTick-backup.csv');
        expect(r).toMatchObject({ source: 'TickTick', skipped: 1, lossy: 0 });
        expect(r.tasks[0]).toMatchObject({ key: 'a1', list: 'Work', title: 'Ship it', notes: 'details', tags: ['urgent', 'q4'], priority: 5, dueMs: Date.UTC(2026, 9, 5, 14), isAllDay: false, timeZone: 'Europe/London', repeatFlag: 'RRULE:FREQ=WEEKLY;INTERVAL=1', reminders: ['TRIGGER:PT0S', 'TRIGGER:-PT5M'], status: 'open' });
        expect(r.tasks.map((t) => t.status)).toEqual(['open', 'done', 'wontdo']);
    });
    it('splits checklist content into items and notes, and links subtasks', () => {
        const r = readImport(ticktick({ Title: 'Pack', Kind: 'CHECKLIST', Content: 'for the trip\n▫socks\n▪passport', taskId: 'p' }, { Title: 'Book hotel', taskId: 'c', parentId: 'p' }), 'b.csv');
        expect(r.tasks[0]).toMatchObject({ notes: 'for the trip', items: [{ title: 'socks', done: false }, { title: 'passport', done: true }] });
        expect(r.tasks[1]?.parentKey).toBe('p');
    });
    it('keeps an invalid date in the notes and sets no deadline', () => {
        const r = readImport(ticktick({ Title: 'Bad', Content: 'body', 'Due Date': '2026-02-30T10:00:00+0000', taskId: 'b' }), 'b.csv');
        expect(r.lossy).toBe(1);
        expect(r.tasks[0]).toMatchObject({ dueMs: null, notes: 'body\n\nTickTick due date: 2026-02-30T10:00:00+0000' });
    });
    it('counts a repeat rule it does not understand and drops it', () => {
        const r = readImport(ticktick({ Title: 'Odd', Repeat: 'ERULE:NAME=CUSTOM', taskId: 'x' }), 'b.csv');
        expect(r.lossy).toBe(1);
        expect(r.tasks[0]?.repeatFlag).toBe('');
    });
});
describe('readImport: Todoist CSV', () => {
    const HEAD = ['TYPE', 'CONTENT', 'DESCRIPTION', 'PRIORITY', 'INDENT', 'AUTHOR', 'RESPONSIBLE', 'DATE', 'DATE_LANG', 'TIMEZONE'];
    const row = (type, content, o = {}) => [type, content, o.desc ?? '', o.pri ?? '', o.indent ?? '', '', '', o.date ?? '', 'en', ''];
    it('names the list after the file, maps priority, labels, notes, subtasks and dates', () => {
        const text = csv(HEAD, row('section', 'Week 1'), row('task', 'Draft brief @work', { desc: 'goals', pri: '1', indent: '1', date: '2026-10-05' }), row('note', 'a comment'), row('task', 'Outline', { pri: '4', indent: '2', date: 'tomorrow' }), row('task', 'Weekly review', { indent: '1', date: 'every monday' }));
        const r = readImport(text, 'Launch.csv', NOW);
        expect(r).toMatchObject({ source: 'Todoist', skipped: 1, lossy: 1 });
        expect(r.tasks[0]).toMatchObject({ list: 'Launch', title: 'Draft brief', tags: ['work'], priority: 5, notes: 'goals\n\na comment', dueMs: new Date(2026, 9, 5).getTime(), isAllDay: true });
        expect(r.tasks[1]).toMatchObject({ parentKey: r.tasks[0].key, priority: 0, dueMs: new Date(2026, 9, 22).getTime() });
        expect(r.tasks[2]).toMatchObject({ dueMs: null, notes: 'Todoist date: every monday' });
    });
    it('keeps an invalid ISO date in the notes rather than letting quick add roll it over', () => {
        const r = readImport(csv(HEAD, row('task', 'Leap', { date: '2026-02-30' })), 'P.csv', NOW);
        expect(r.lossy).toBe(1);
        expect(r.tasks[0]).toMatchObject({ dueMs: null, notes: 'Todoist date: 2026-02-30' });
    });
    it('gives duplicate and suffix-looking titles distinct keys that repeat on re-import', () => {
        const text = csv(HEAD, row('task', 'Task'), row('task', 'Task'), row('task', 'Task#2'), row('task', 'a|b'));
        const once = readImport(text, 'Project.csv', NOW).tasks.map((t) => t.key);
        expect(new Set(once).size).toBe(4);
        expect(readImport(text, 'Project.csv', NOW).tasks.map((t) => t.key)).toEqual(once);
    });
    it('imports every one of them, and nothing on a second import', async () => {
        const text = csv(HEAD, row('task', 'Task'), row('task', 'Task'), row('task', 'Task#2'));
        const api = new MemoryAdapter();
        expect((await applyImport(api, readImport(text, 'Project.csv', NOW))).tasks).toBe(3);
        expect((await applyImport(api, readImport(text, 'Project.csv', NOW))).existing).toBe(3);
        expect((await api.snapshot()).tasks.map((t) => t.title).sort()).toEqual(['Task', 'Task', 'Task#2']);
    });
});
describe('readImport: generic CSV', () => {
    it('reads aliased columns, priority words, completion and duplicates as separate tasks', () => {
        const text = csv(['List', 'Title', 'Notes', 'Due', 'Priority', 'Tags', 'Completed'], ['Home', 'Milk', 'semi', '2026-10-05', 'high', 'flagged', ''], ['Home', 'Milk', 'semi', '2026-10-05', 'low', '', 'true'], ['', 'Call', '', 'soon', '', '', '']);
        const r = readImport(text, 'reminders.csv');
        expect(r).toMatchObject({ source: 'CSV', skipped: 0, lossy: 1 });
        expect(r.tasks[2]).toMatchObject({ dueMs: null, notes: 'Due: soon' });
        expect(r.tasks[0]).toMatchObject({ list: 'Home', title: 'Milk', notes: 'semi', priority: 5, tags: ['flagged'], isAllDay: true, status: 'open' });
        expect(r.tasks[1]).toMatchObject({ priority: 1, status: 'done' });
        expect(new Set(r.tasks.map((t) => t.key)).size).toBe(3);
    });
    it('refuses a file it does not recognise', () => {
        expect(() => readImport('foo,bar\n1,2', 'x.csv')).toThrow(/Title column/);
    });
});
describe('readImport: .ics', () => {
    const event = (...lines) => ['BEGIN:VEVENT', ...lines, 'END:VEVENT'].join('\r\n');
    const cal = (...events) => ['BEGIN:VCALENDAR', ...events, 'END:VCALENDAR'].join('\r\n');
    // A repeating event with one instance moved: both share the UID.
    const moved = cal(event('UID:r1', 'SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'RRULE:FREQ=WEEKLY'), event('UID:r1', 'RECURRENCE-ID:20261012T090000Z', 'SUMMARY:Standup', 'DTSTART:20261013T090000Z'));
    const movedB = event('UID:r1', 'RECURRENCE-ID:20261019T090000Z', 'SUMMARY:Standup', 'DTSTART:20261020T090000Z');
    const master = event('UID:r1', 'SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'RRULE:FREQ=WEEKLY');
    const movedA = event('UID:r1', 'RECURRENCE-ID:20261012T090000Z', 'SUMMARY:Standup', 'DTSTART:20261013T090000Z');
    /** The local calendar days the app draws Standup on, through October. */
    const shownDays = async (api) => buildEvents((await api.snapshot()).tasks.filter((t) => t.title === 'Standup'), false, new Date(2026, 10, 1).getTime())
        .map((e) => dayKey(e.startDay))
        .sort();
    const day = (iso) => dayKey(startOfDay(Date.parse(iso)));
    it('shows a moved instance on its new day and not on the day it replaced', async () => {
        const api = new MemoryAdapter();
        await applyImport(api, readImport(moved, 'cal.ics'));
        expect(await shownDays(api)).toEqual([day('2026-10-05T09:00Z'), day('2026-10-13T09:00Z'), day('2026-10-19T09:00Z'), day('2026-10-26T09:00Z')].sort());
    });
    it('carries EXDATEs, comma lists included, and counts one it cannot read', async () => {
        const file = cal(event('UID:x1', 'SUMMARY:Standup', 'DTSTART:20261005T090000Z', 'RRULE:FREQ=WEEKLY', 'EXDATE:20261012T090000Z,20261019T090000Z', 'EXDATE:not-a-date'));
        const r = readImport(file, 'cal.ics');
        expect(r.lossy).toBe(1);
        const api = new MemoryAdapter();
        await applyImport(api, r);
        expect(await shownDays(api)).toEqual([day('2026-10-05T09:00Z'), day('2026-10-26T09:00Z')].sort());
    });
    it('excludes the occurrence a cancelled instance stands for', async () => {
        const file = cal(master, event('UID:r1', 'RECURRENCE-ID:20261012T090000Z', 'STATUS:CANCELLED', 'SUMMARY:Standup', 'DTSTART:20261012T090000Z'));
        const api = new MemoryAdapter();
        await applyImport(api, readImport(file, 'cal.ics'));
        expect(await shownDays(api)).not.toContain(day('2026-10-12T09:00Z'));
    });
    it('keeps each instance by its RECURRENCE-ID when instances are added or reordered, through either import', async () => {
        const api = new MemoryAdapter();
        await applyImport(api, readImport(cal(master, movedA), 'cal.ics')); // as the calendar page does
        const a = (await api.snapshot()).tasks.find((t) => t.dueMs === Date.UTC(2026, 9, 13, 9));
        await api.updateTask(a.id, { notes: 'mine' });
        const n = await applyImport(api, readImport(cal(master, movedB, movedA), 'other.ics')); // as Settings does
        expect(n.tasks).toBe(1);
        const tasks = (await api.snapshot()).tasks;
        expect(tasks).toHaveLength(3);
        expect(tasks.find((t) => t.id === a.id)).toMatchObject({ dueMs: Date.UTC(2026, 9, 13, 9), notes: 'mine' });
        expect(tasks.find((t) => t.dueMs === Date.UTC(2026, 9, 20, 9))).toBeDefined();
        expect((await applyImport(api, readImport(cal(movedA, master, movedB), 'cal.ics'))).existing).toBe(3);
    });
    it('gives each instance a key that does not depend on its place in the file', () => {
        const keys = (text) => Object.fromEntries(readImport(text, 'cal.ics').tasks.map((t) => [t.key, t.dueMs]));
        expect(keys(cal(master, movedA, movedB))).toEqual(keys(cal(movedB, master, movedA)));
    });
    it('matches a file imported before by bare UID, and adds only the instance that was dropped then', async () => {
        const api = new MemoryAdapter();
        const inbox = (await api.snapshot()).inboxId;
        await api.createTask({ id: derivedId('import|calendar file|r1'), listId: inbox, title: 'Standup', dueMs: Date.UTC(2026, 9, 5, 9), repeatFlag: 'RRULE:FREQ=WEEKLY' });
        expect(await applyImport(api, readImport(moved, 'cal.ics'))).toEqual({ tasks: 1, lists: 0, existing: 0, resumed: 1 }); // its exception is added: it was untouched
        expect((await api.snapshot()).tasks).toHaveLength(2);
        expect(await shownDays(api)).not.toContain(day('2026-10-12T09:00Z'));
    });
    /** What #829 left: each row of `rows` (in file order) as `[uid, n]`, with its own due date and no exceptions. */
    async function written829(api, rows) {
        const inbox = (await api.snapshot()).inboxId;
        const seen = new Map();
        const ids = [];
        for (const r of rows) {
            const n = (seen.get(r.uid) ?? 0) + 1;
            seen.set(r.uid, n);
            const id = derivedId(`import|calendar file|${JSON.stringify([r.uid, n])}`);
            await api.createTask({ id, listId: inbox, title: r.title, dueMs: r.dueMs, repeatFlag: r.repeat ? 'RRULE:FREQ=WEEKLY' : '' });
            ids.push(id);
        }
        return ids;
    }
    const m829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 5, 9), repeat: true };
    const a829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 13, 9) };
    it('matches tasks #829 imported, adds the master its exceptions, and adds nothing else', async () => {
        const api = new MemoryAdapter();
        await written829(api, [m829, a829]);
        expect(await applyImport(api, readImport(cal(master, movedA), 'cal.ics'))).toEqual({ tasks: 0, lists: 0, existing: 1, resumed: 1 });
        expect((await api.snapshot()).tasks).toHaveLength(2);
        expect(await shownDays(api)).toEqual([day('2026-10-05T09:00Z'), day('2026-10-13T09:00Z'), day('2026-10-19T09:00Z'), day('2026-10-26T09:00Z')].sort());
    });
    it('matches tasks #829 imported by due date when the file is reordered and grows, keeping edits', async () => {
        const api = new MemoryAdapter();
        const [, aId] = await written829(api, [m829, a829]);
        await api.updateTask(aId, { notes: 'mine' });
        const n = await applyImport(api, readImport(cal(movedB, movedA, master), 'cal.ics'));
        expect(n).toEqual({ tasks: 1, lists: 0, existing: 1, resumed: 1 });
        const tasks = (await api.snapshot()).tasks;
        expect(tasks).toHaveLength(3);
        expect(tasks.find((t) => t.id === aId)).toMatchObject({ dueMs: Date.UTC(2026, 9, 13, 9), notes: 'mine' });
    });
    it('matches a task #829 imported after an instance before it was removed from the file', async () => {
        const api = new MemoryAdapter();
        const b829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 20, 9) };
        const [, , bId] = await written829(api, [m829, a829, b829]);
        const n = await applyImport(api, readImport(cal(master, movedB), 'cal.ics'));
        expect(n.tasks).toBe(0);
        expect((await api.snapshot()).tasks.map((t) => t.id)).toContain(bId);
    });
    it('matches tasks #829 imported whose due dates were edited since, by title and repeat, keeping edits', async () => {
        const api = new MemoryAdapter();
        const [mId, aId] = await written829(api, [m829, a829]);
        await api.updateTask(mId, { dueMs: Date.UTC(2026, 9, 6, 9) });
        await api.updateTask(aId, { dueMs: Date.UTC(2026, 9, 14, 9), notes: 'mine' });
        expect(await applyImport(api, readImport(cal(movedA, master), 'cal.ics'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 });
        const tasks = (await api.snapshot()).tasks;
        expect(tasks).toHaveLength(2);
        expect(tasks.find((t) => t.id === aId)).toMatchObject({ dueMs: Date.UTC(2026, 9, 14, 9), notes: 'mine' });
    });
    it('matches a task imported by bare UID whose due date was edited since', async () => {
        const api = new MemoryAdapter();
        const inbox = (await api.snapshot()).inboxId;
        const id = derivedId('import|calendar file|r1');
        await api.createTask({ id, listId: inbox, title: 'Standup', dueMs: Date.UTC(2026, 9, 5, 9), repeatFlag: 'RRULE:FREQ=WEEKLY' });
        await api.updateTask(id, { dueMs: Date.UTC(2026, 9, 7, 9) });
        expect(await applyImport(api, readImport(moved, 'cal.ics'))).toEqual({ tasks: 1, lists: 0, existing: 1, resumed: 0 }); // the instance dropped then
        expect((await api.snapshot()).tasks).toHaveLength(2);
    });
    it('matches the one edited task left when the others still have their due dates', async () => {
        const api = new MemoryAdapter();
        const b829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 20, 9) };
        const [, , bId] = await written829(api, [m829, a829, b829]);
        await api.updateTask(bId, { dueMs: Date.UTC(2026, 9, 21, 9) });
        expect((await applyImport(api, readImport(cal(master, movedA, movedB), 'cal.ics'))).tasks).toBe(0);
    });
    it('matches look-alike tasks whose due dates were all edited, one task each, keeping edits', async () => {
        const api = new MemoryAdapter();
        const b829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 20, 9) };
        const [, aId, bId] = await written829(api, [m829, a829, b829]);
        await api.updateTask(aId, { dueMs: Date.UTC(2026, 9, 14, 9) });
        await api.updateTask(bId, { dueMs: Date.UTC(2026, 9, 21, 9) });
        expect(await applyImport(api, readImport(cal(master, movedA, movedB), 'cal.ics'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 1 });
        const tasks = (await api.snapshot()).tasks;
        expect(tasks).toHaveLength(3);
        expect(tasks.find((t) => t.id === aId).dueMs).toBe(Date.UTC(2026, 9, 14, 9));
        expect(tasks.find((t) => t.id === bId).dueMs).toBe(Date.UTC(2026, 9, 21, 9));
    });
    it('does not guess which look-alike instance is new when there are more of them than edited tasks', async () => {
        const api = new MemoryAdapter();
        const b829 = { uid: 'r1', title: 'Standup', dueMs: Date.UTC(2026, 9, 20, 9) };
        const [, aId, bId] = await written829(api, [m829, a829, b829]);
        await api.updateTask(aId, { dueMs: Date.UTC(2026, 9, 14, 9) });
        await api.updateTask(bId, { dueMs: Date.UTC(2026, 9, 21, 9) });
        const movedC = event('UID:r1', 'RECURRENCE-ID:20261026T090000Z', 'SUMMARY:Standup', 'DTSTART:20261027T090000Z');
        expect(await applyImport(api, readImport(cal(master, movedA, movedB, movedC), 'cal.ics'))).toEqual({ tasks: 3, lists: 0, existing: 0, resumed: 1 });
    });
    it('matches tasks #829 imported whose titles and due dates were both edited since', async () => {
        const api = new MemoryAdapter();
        const [mId, aId] = await written829(api, [m829, a829]);
        await api.updateTask(mId, { title: 'Team standup', dueMs: Date.UTC(2026, 9, 6, 9) });
        await api.updateTask(aId, { title: 'Standup (moved)', dueMs: Date.UTC(2026, 9, 14, 9) });
        expect(await applyImport(api, readImport(cal(movedA, master), 'cal.ics'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 });
        expect((await api.snapshot()).tasks.map((t) => t.title).sort()).toEqual(['Standup (moved)', 'Team standup']);
    });
    it('matches a task #829 imported whose repeat, title and due date were all edited since', async () => {
        const api = new MemoryAdapter();
        const [mId] = await written829(api, [m829, a829]);
        await api.updateTask(mId, { title: 'One-off sync', repeatFlag: '', dueMs: Date.UTC(2026, 9, 6, 9) });
        expect(await applyImport(api, readImport(cal(master, movedA), 'cal.ics'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 });
        expect((await api.snapshot()).tasks.find((t) => t.id === mId)).toMatchObject({ title: 'One-off sync', repeatFlag: '' });
    });
    it('does not take an untouched task for a new instance when the one it was is gone from the file', async () => {
        const api = new MemoryAdapter();
        const [, aId] = await written829(api, [m829, a829]);
        expect(await applyImport(api, readImport(cal(master, movedB), 'cal.ics'))).toEqual({ tasks: 1, lists: 0, existing: 0, resumed: 1 });
        expect((await api.snapshot()).tasks.find((t) => t.id === aId).dueMs).toBe(a829.dueMs);
    });
    it('writes a row merged into an earlier task to that task, and gives that task to no other row', async () => {
        const api = new MemoryAdapter();
        const [, aId] = await written829(api, [m829, a829]);
        await api.updateTask(aId, { dueMs: Date.UTC(2026, 9, 14, 9) });
        const movedAId = derivedId(`import|calendar file|${JSON.stringify(['r1', String(Date.UTC(2026, 9, 12, 9)), 1])}`);
        await recordMerge(api, movedAId, aId); // its duplicate was merged into the #829 task, then purged
        expect(await applyImport(api, readImport(cal(master, movedA, movedB), 'cal.ics'))).toEqual({ tasks: 1, lists: 0, existing: 1, resumed: 1 });
        expect((await api.snapshot()).tasks.find((t) => t.dueMs === Date.UTC(2026, 9, 20, 9))).toBeDefined();
    });
    it('leaves an untouched task a duplicate was merged into as it is: no status or exceptions from the file', async () => {
        const api = new MemoryAdapter();
        const inbox = (await api.snapshot()).inboxId;
        const keep = await api.createTask({ listId: inbox, title: 'Standup' }); // the user's own, still at etag 1
        await applyImport(api, readImport(cal(master, movedA), 'cal.ics'));
        const duplicate = (await api.snapshot()).tasks.find((t) => t.repeatFlag !== '');
        await recordMerge(api, duplicate.id, keep.id);
        await api.purgeTask(duplicate.id);
        expect((await applyImport(api, readImport(cal(master, movedA), 'cal.ics'))).resumed).toBe(0);
        expect((await api.snapshot()).tasks.find((t) => t.id === keep.id)).toMatchObject({ etag: '1', exDates: [] });
    });
    it('imports an event with more exceptions than a task can hold, keeping the latest', async () => {
        const exdates = Array.from({ length: 1500 }, (_, i) => new Date(Date.UTC(2026, 9, 6) + i * 86_400_000).toISOString().replace(/[-:]|\.\d{3}/g, ''));
        const api = new MemoryAdapter();
        await applyImport(api, readImport(cal(event('UID:d1', 'SUMMARY:Daily', 'DTSTART:20261005T090000Z', 'RRULE:FREQ=DAILY', `EXDATE:${exdates.join(',')}`)), 'cal.ics'));
        const task = (await api.snapshot()).tasks.find((t) => t.title === 'Daily');
        expect(task.exDates).toHaveLength(1000);
        expect(task.exDates.at(-1)).toBe(Date.parse('2026-10-06T00:00:00Z') + 1499 * 86_400_000);
    });
    it('does not add exceptions to an earlier-imported master the user has since edited', async () => {
        const api = new MemoryAdapter();
        const inbox = (await api.snapshot()).inboxId;
        const id = derivedId('import|calendar file|r1');
        await api.createTask({ id, listId: inbox, title: 'Standup', dueMs: Date.UTC(2026, 9, 5, 9), repeatFlag: 'RRULE:FREQ=WEEKLY' });
        await api.updateTask(id, { notes: 'mine' });
        expect((await applyImport(api, readImport(moved, 'cal.ics'))).resumed).toBe(0);
        expect((await api.snapshot()).tasks.find((t) => t.id === id)).toMatchObject({ notes: 'mine', exDates: [] });
    });
    it('reads VTODOs through the calendar parser into the Inbox', () => {
        const text = ['BEGIN:VCALENDAR', 'BEGIN:VTODO', 'UID:r1', 'SUMMARY:Renew passport', 'DUE;VALUE=DATE:20261105', 'END:VTODO', 'END:VCALENDAR'].join('\r\n');
        const r = readImport(text, 'Reminders.ics');
        expect(r.tasks).toHaveLength(1);
        expect(r.tasks[0]).toMatchObject({ key: '["r1","",1]', list: '', title: 'Renew passport', isAllDay: true });
        const due = new Date(2026, 10, 5).getTime();
        expect(r.tasks[0].legacy).toEqual(expect.arrayContaining([{ key: '["r1",1]', dueMs: due }, { key: '["r1",2]', dueMs: due }, { key: 'r1', dueMs: due }]));
    });
});
describe('applyImport', () => {
    const file = ticktick({ 'List Name': 'Work', Title: 'Parent', taskId: 'p', Priority: '3' }, { 'List Name': 'Elsewhere', Title: 'Child', taskId: 'c', parentId: 'p' }, { 'List Name': 'Work', Title: 'Grandchild', taskId: 'g', parentId: 'c' }, { 'List Name': 'inbox', Title: 'Closed', taskId: 'd', Status: '1' }, { 'List Name': 'Work', Title: 'Pack', Kind: 'CHECKLIST', Content: '▫socks', taskId: 'k' });
    it('creates missing lists by name, nests subtasks one level in their parent list, and sets status', async () => {
        const api = new MemoryAdapter();
        const n = await applyImport(api, readImport(file, 'b.csv'));
        expect(n).toEqual({ tasks: 5, lists: 1, existing: 0, resumed: 0 });
        const snap = await api.snapshot();
        const by = (title) => snap.tasks.find((t) => t.title === title);
        const work = snap.lists.find((l) => l.name === 'Work');
        expect(by('Parent')).toMatchObject({ listId: work.id, priority: 3, parentId: null });
        expect(by('Child')).toMatchObject({ listId: work.id, parentId: by('Parent').id });
        expect(by('Grandchild')).toMatchObject({ listId: work.id, parentId: by('Parent').id });
        expect(by('Closed')).toMatchObject({ listId: snap.inboxId, status: 'done' });
        expect(by('Pack')).toMatchObject({ kind: 'checklist', items: [expect.objectContaining({ title: 'socks', done: false })] });
    });
    it('is idempotent, even after an imported list is renamed', async () => {
        const api = new MemoryAdapter();
        await applyImport(api, readImport(file, 'b.csv'));
        const work = (await api.snapshot()).lists.find((l) => l.name === 'Work');
        await api.updateList(work.id, { name: 'Job' });
        expect(await applyImport(api, readImport(file, 'b.csv'))).toEqual({ tasks: 0, lists: 0, existing: 5, resumed: 0 });
        expect((await api.snapshot()).lists.map((l) => l.name).sort()).toEqual(['Inbox', 'Job']);
    });
    /** A MemoryAdapter whose next status write fails, as a dropped request would. */
    const failingStatusOnce = () => {
        const api = new MemoryAdapter();
        const update = api.updateTask.bind(api);
        let failed = false;
        api.updateTask = async (id, patch, etag) => {
            if (patch.status !== undefined && !failed) {
                failed = true;
                throw new Error('network down');
            }
            return update(id, patch, etag);
        };
        return api;
    };
    const closed = ticktick({ Title: 'Closed', taskId: 'd', Status: '1' }, { Title: 'Later', taskId: 'e' });
    it('finishes a status write lost between create and update on the next import', async () => {
        const api = failingStatusOnce();
        await expect(applyImport(api, readImport(closed, 'b.csv'))).rejects.toThrow('network down');
        expect((await api.snapshot()).tasks.find((t) => t.title === 'Closed')?.status).toBe('open');
        expect(await applyImport(api, readImport(closed, 'b.csv'))).toEqual({ tasks: 1, lists: 0, existing: 0, resumed: 1 });
        expect((await api.snapshot()).tasks.find((t) => t.title === 'Closed')?.status).toBe('done');
        expect(await applyImport(api, readImport(closed, 'b.csv'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 });
    });
    it('leaves a task edited since the failed write alone, rather than overwrite the edit', async () => {
        const api = failingStatusOnce();
        await expect(applyImport(api, readImport(closed, 'b.csv'))).rejects.toThrow();
        const task = (await api.snapshot()).tasks.find((t) => t.title === 'Closed');
        await api.updateTask(task.id, { title: 'Closed, renamed' });
        expect((await applyImport(api, readImport(closed, 'b.csv'))).resumed).toBe(0);
        expect((await api.snapshot()).tasks.find((t) => t.id === task.id)).toMatchObject({ title: 'Closed, renamed', status: 'open' });
    });
    it('does not reopen or re-close a task the user changed after a clean import', async () => {
        const api = new MemoryAdapter();
        await applyImport(api, readImport(closed, 'b.csv'));
        const task = (await api.snapshot()).tasks.find((t) => t.title === 'Closed');
        await api.updateTask(task.id, { status: 'open' });
        expect((await applyImport(api, readImport(closed, 'b.csv'))).resumed).toBe(0);
        expect((await api.snapshot()).tasks.find((t) => t.id === task.id)?.status).toBe('open');
    });
    it('treats a parent cycle as top-level tasks instead of failing', async () => {
        const api = new MemoryAdapter();
        const n = await applyImport(api, readImport(ticktick({ Title: 'A', taskId: 'a', parentId: 'b' }, { Title: 'B', taskId: 'b', parentId: 'a' }), 'b.csv'));
        expect(n.tasks).toBe(2);
    });
});
describe('applyImport: files imported before the key change (#821)', () => {
    const HEAD = ['TYPE', 'CONTENT', 'DESCRIPTION', 'PRIORITY', 'INDENT', 'AUTHOR', 'RESPONSIBLE', 'DATE', 'DATE_LANG', 'TIMEZONE'];
    const task = (content, indent = '1') => ['task', content, '', '', indent, '', '', '', 'en', ''];
    const todoist = csv(HEAD, task('Task'), task('Task'), task('Task#2'), task('Parent'), task('Child', '2'));
    /** The id #821 gave a row: `derivedId("import|<source>|<legacy key>")`. */
    const oldId = (source, key) => derivedId(`import|${source}|${key}`);
    /** What #821 left after importing `todoist`: the colliding `Task#2` row was never created. */
    async function legacyTodoist(api) {
        const list = await api.createList({ id: derivedId('import|list|p'), name: 'P' });
        await api.createTask({ id: oldId('Todoist', 'P|Task'), listId: list.id, title: 'Task' });
        await api.createTask({ id: oldId('Todoist', 'P|Task#2'), listId: list.id, title: 'Task' });
        await api.createTask({ id: oldId('Todoist', 'P|Parent'), listId: list.id, title: 'Parent' });
        await api.createTask({ id: oldId('Todoist', 'P|Child'), listId: list.id, title: 'Child', parentId: oldId('Todoist', 'P|Parent') });
        return list.id;
    }
    it('replays the old keys in file order, collisions included', () => {
        const legacy = legacyKeys();
        expect(['P|Task', 'P|Task', 'P|Task#2'].map((base) => legacy(base))).toEqual(['P|Task', 'P|Task#2', 'P|Task#2']);
    });
    it('gives a colliding old key to the row #821 wrote first (top-level before subtasks), not the first in the file', async () => {
        // #821 wrote Parent and the top-level Task#2 (key P|Task#2), then the first Task child (P|Task);
        // the second Task child (also P|Task#2) collided and was never written.
        const file = csv(HEAD, task('Parent'), task('Task', '2'), task('Task', '2'), task('Task#2'));
        const api = new MemoryAdapter();
        const list = await api.createList({ id: derivedId('import|list|p'), name: 'P' });
        await api.createTask({ id: oldId('Todoist', 'P|Parent'), listId: list.id, title: 'Parent' });
        await api.createTask({ id: oldId('Todoist', 'P|Task#2'), listId: list.id, title: 'Task#2' });
        await api.createTask({ id: oldId('Todoist', 'P|Task'), listId: list.id, title: 'Task', parentId: oldId('Todoist', 'P|Parent') });
        await api.updateTask(oldId('Todoist', 'P|Task#2'), { notes: 'mine' });
        expect(await applyImport(api, readImport(file, 'P.csv', NOW))).toEqual({ tasks: 1, lists: 0, existing: 3, resumed: 0 });
        const tasks = (await api.snapshot()).tasks;
        expect(tasks).toHaveLength(4);
        expect(tasks.filter((t) => t.title === 'Task#2')).toEqual([expect.objectContaining({ id: oldId('Todoist', 'P|Task#2'), parentId: null, notes: 'mine' })]);
        expect(tasks.filter((t) => t.title === 'Task').map((t) => t.parentId)).toEqual([oldId('Todoist', 'P|Parent'), oldId('Todoist', 'P|Parent')]);
        expect(await applyImport(api, readImport(file, 'P.csv', NOW))).toEqual({ tasks: 0, lists: 0, existing: 4, resumed: 0 });
    });
    it('matches each old task once, imports the row #821 lost, and keeps edits and parent links', async () => {
        const api = new MemoryAdapter();
        await legacyTodoist(api);
        await api.updateTask(oldId('Todoist', 'P|Task'), { title: 'Task (edited)' });
        expect(await applyImport(api, readImport(todoist, 'P.csv', NOW))).toEqual({ tasks: 1, lists: 0, existing: 4, resumed: 0 });
        const snap = await api.snapshot();
        const mine = snap.tasks.filter((t) => t.title !== 'Inbox');
        expect(mine.map((t) => t.title).sort()).toEqual(['Child', 'Parent', 'Task', 'Task (edited)', 'Task#2']);
        expect(mine.find((t) => t.title === 'Child')?.parentId).toBe(oldId('Todoist', 'P|Parent'));
        expect(await applyImport(api, readImport(todoist, 'P.csv', NOW))).toEqual({ tasks: 0, lists: 0, existing: 5, resumed: 0 });
        expect((await api.snapshot()).tasks).toHaveLength(5);
    });
    it('finishes a completion #821 lost, and leaves an old task the user edited alone', async () => {
        const generic = csv(['List', 'Title', 'Completed'], ['', 'Lost close', 'true'], ['', 'Edited', 'true']);
        const api = new MemoryAdapter();
        const inbox = (await api.snapshot()).inboxId;
        await api.createTask({ id: oldId('CSV', '|Lost close|'), listId: inbox, title: 'Lost close' }); // its status write failed
        await api.createTask({ id: oldId('CSV', '|Edited|'), listId: inbox, title: 'Edited' });
        await api.updateTask(oldId('CSV', '|Edited|'), { notes: 'mine' });
        expect(await applyImport(api, readImport(generic, 'x.csv'))).toEqual({ tasks: 0, lists: 0, existing: 1, resumed: 1 });
        const snap = await api.snapshot();
        expect(snap.tasks).toHaveLength(2);
        expect(snap.tasks.find((t) => t.title === 'Lost close')?.status).toBe('done');
        expect(snap.tasks.find((t) => t.title === 'Edited')).toMatchObject({ status: 'open', notes: 'mine' });
        expect(await applyImport(api, readImport(generic, 'x.csv'))).toEqual({ tasks: 0, lists: 0, existing: 2, resumed: 0 });
    });
    it('creates fresh tasks for rows whose old task the user has since deleted', async () => {
        const api = new MemoryAdapter();
        await legacyTodoist(api);
        await api.purgeTask(oldId('Todoist', 'P|Parent')); // takes Child with it
        const n = await applyImport(api, readImport(todoist, 'P.csv', NOW));
        expect(n).toEqual({ tasks: 3, lists: 0, existing: 2, resumed: 0 });
        const snap = await api.snapshot();
        const parent = snap.tasks.find((t) => t.title === 'Parent');
        expect(snap.tasks.find((t) => t.title === 'Child')?.parentId).toBe(parent.id);
    });
});

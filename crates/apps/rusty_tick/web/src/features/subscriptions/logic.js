import { storedExDates } from '@/lib/ics';
/** A feed is read at most this many events deep, so `items` stays inside a doc's size limit. */
export const MAX_ITEMS = 500;
export function asSubscriptionBody(body) {
    if (typeof body !== 'object' || body === null)
        return null;
    const o = body;
    if (typeof o.name !== 'string' || typeof o.url !== 'string' || typeof o.listId !== 'string' || !o.name.trim() || !o.url.trim())
        return null;
    const items = {};
    if (typeof o.items === 'object' && o.items !== null) {
        for (const [uid, id] of Object.entries(o.items))
            if (typeof id === 'string')
                items[uid] = id;
    }
    return { name: o.name, url: o.url, listId: o.listId, items, syncedMs: typeof o.syncedMs === 'number' ? o.syncedMs : 0 };
}
/**
 * The key a feed event's task is stored under: its UID, plus its
 * `RECURRENCE-ID` for a moved instance, so the instances of a repeating event
 * each keep a task. A master or one-off keeps the bare UID, as stored before.
 */
export const itemKey = (e) => (e.recurrenceId === null ? e.uid : JSON.stringify([e.uid, e.recurrenceId]));
/** The task fields a new event's task is created with. */
export const sourceFields = (t) => ({
    title: t.title,
    notes: t.notes,
    startMs: t.startMs,
    dueMs: t.dueMs,
    isAllDay: t.isAllDay,
    timeZone: t.timeZone,
    repeatFlag: t.repeatFlag,
});
/**
 * The task fields a feed owns; a sync overwrites these and nothing else. With
 * the repeat rule it owns the rule's exceptions, so an occurrence the feed
 * moved or cancelled is not shown beside its replacement.
 */
const feedFields = (t) => ({
    ...sourceFields(t),
    exDates: storedExDates(t.exDates),
});
const same = (a, b) => (Array.isArray(a) && Array.isArray(b) ? a.length === b.length && a.every((x, i) => x === b[i]) : a === b);
/**
 * What to do so the tasks match `feed`, event by `itemKey`:
 * - a new event creates a task (its exceptions, which a create cannot carry,
 *   are written right after; see `syncSubscription`);
 * - a known event updates the fields the feed owns, if any changed (a task the
 *   user deleted stays deleted: it is not brought back);
 * - an event gone from the feed trashes its task.
 */
export function planSync(feed, items, tasks) {
    const plan = { create: [], update: [], trash: [], items: {} };
    const seen = new Set();
    for (const event of feed.slice(0, MAX_ITEMS)) {
        const key = itemKey(event);
        if (seen.has(key))
            continue;
        seen.add(key);
        const taskId = items[key];
        if (taskId === undefined) {
            plan.create.push(event);
            continue;
        }
        plan.items[key] = taskId;
        const task = tasks[taskId];
        if (!task || task.deletedMs !== null)
            continue;
        const want = feedFields(event);
        const patch = {};
        for (const field of Object.keys(want))
            if (!same(task[field], want[field]))
                patch[field] = want[field];
        if (Object.keys(patch).length > 0)
            plan.update.push({ taskId, patch: patch });
    }
    for (const [key, taskId] of Object.entries(items)) {
        if (seen.has(key))
            continue;
        const task = tasks[taskId];
        if (task && task.deletedMs === null)
            plan.trash.push(taskId);
    }
    return plan;
}
/** Whether a subscription is due an automatic refresh. */
export const isStale = (s, now, maxAgeMs) => now - s.syncedMs > maxAgeMs;

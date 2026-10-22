import { addDays, dayKey, diffDays, monthName, startOfDay, startOfWeek, weekdayName } from '@/lib/date';
export const RANGE_LABEL = {
    all: 'All Dates',
    today: 'Today',
    yesterday: 'Yesterday',
    week: 'This Week',
    month: 'This Month',
};
/** Whether `ms` falls in `range`, relative to `now` (and the week's first day). */
export function inRange(ms, range, now, weekStart) {
    const today = startOfDay(now);
    switch (range) {
        case 'all':
            return true;
        case 'today':
            return startOfDay(ms) === today;
        case 'yesterday':
            return startOfDay(ms) === addDays(today, -1);
        case 'week': {
            const start = startOfWeek(now, weekStart);
            return ms >= start && ms < addDays(start, 7);
        }
        case 'month': {
            const a = new Date(ms);
            const b = new Date(now);
            return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();
        }
    }
}
/** `Today`, `Yesterday`, `Mon, Sep 28`, and with the year when it is not this one. */
export function dayLabel(day, now) {
    const d = diffDays(now, day);
    if (d === 0)
        return 'Today';
    if (d === -1)
        return 'Yesterday';
    const date = new Date(day);
    const base = `${weekdayName(date.getDay())}, ${monthName(date.getMonth())} ${date.getDate()}`;
    return date.getFullYear() === new Date(now).getFullYear() ? base : `${base}, ${date.getFullYear()}`;
}
/** Group tasks by the local day of `at(task)`, newest day first, newest task first within it. */
export function groupByDay(tasks, at, now) {
    const groups = new Map();
    for (const t of tasks) {
        const ms = at(t);
        if (ms === null)
            continue;
        const key = dayKey(ms);
        const g = groups.get(key) ?? { day: startOfDay(ms), tasks: [] };
        g.tasks.push({ t, at: ms });
        groups.set(key, g);
    }
    return [...groups.entries()]
        .sort(([a], [b]) => (a < b ? 1 : -1))
        .map(([key, g]) => ({ key, label: dayLabel(g.day, now), tasks: g.tasks.sort((x, y) => y.at - x.at).map((x) => x.t) }));
}
/** Completed, not trashed, matching the filters, grouped by completion day. */
export function completedGroups(tasks, filter, now, weekStart) {
    const picked = tasks.filter((t) => t.status !== 'open' &&
        t.deletedMs === null &&
        t.completedMs !== null &&
        (filter.listId === null || t.listId === filter.listId) &&
        inRange(t.completedMs, filter.range, now, weekStart));
    return groupByDay(picked, (t) => t.completedMs, now);
}
/** Trashed tasks, grouped by the day they were deleted. */
export function trashGroups(tasks, now) {
    return groupByDay(tasks.filter((t) => t.deletedMs !== null), (t) => t.deletedMs, now);
}

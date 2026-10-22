import { addDays, atTime, startOfDay } from '@/lib/date';
/** Columns are made from the view's grouping; "none" has no columns of its own, so use priority. */
export const kanbanGroupBy = (by) => (by === 'none' ? 'priority' : by);
const PRIORITY_OF = { p5: 5, p3: 3, p1: 1, p0: 0 };
/**
 * The patch that moves `task` from column `from` to column `to` of a board
 * grouped by `by`, or `null` if the drop means nothing (same column, or a
 * column that cannot be set: "Overdue").
 */
export function dropPatch(task, by, from, to, now) {
    if (from === to)
        return null;
    switch (by) {
        case 'priority': {
            const p = PRIORITY_OF[to];
            return p === undefined ? null : { priority: p };
        }
        case 'list':
            return { listId: to };
        case 'tag': {
            const tags = task.tags.filter((t) => t !== from);
            if (to !== 'notag' && !tags.includes(to))
                tags.push(to);
            return { tags };
        }
        case 'date': {
            const today = startOfDay(now);
            const day = { today: today, tomorrow: addDays(today, 1), next7: addDays(today, 3), later: addDays(today, 14) }[to];
            if (to === 'nodate')
                return { dueMs: null, startMs: null, isAllDay: false };
            if (day === undefined)
                return null; // Overdue is a state, not a date to pick
            // Keep a time of day if the task had one; otherwise the move makes it all-day.
            if (task.dueMs !== null && !task.isAllDay) {
                const d = new Date(task.dueMs);
                return { dueMs: atTime(day, d.getHours(), d.getMinutes()) };
            }
            return { dueMs: day, isAllDay: true };
        }
    }
}

export const at = (y, m, d, h = 0, min = 0, s = 0, ms = 0) => new Date(y, m - 1, d, h, min, s, ms).getTime();
/** Tuesday 29 Sep 2026, 15:00. */
export const NOW = at(2026, 9, 29, 15);
export const list = (id, name, sortOrder = 0) => ({ id, name, color: null, archived: false, viewMode: 'list', sortType: '', sortOrder, updatedMs: 0, etag: '1' });
let n = 0;
export function task(over = {}) {
    n += 1;
    return {
        id: `t${n}`, listId: 'work', parentId: null, title: `Task ${n}`, notes: '', kind: 'text', status: 'open', priority: 0,
        startMs: null, dueMs: null, isAllDay: false, timeZone: '', reminders: [], repeatFlag: '', exDates: [], items: [], tags: [],
        sortOrder: n, createdMs: 0, updatedMs: 0, completedMs: null, deletedMs: null, etag: '1', ...over,
    };
}
export const done = (completedMs, over = {}) => task({ status: 'done', completedMs, ...over });

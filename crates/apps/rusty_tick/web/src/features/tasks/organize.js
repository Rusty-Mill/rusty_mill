import { addDays, diffDays, startOfDay } from '@/lib/date';
import { matchesFilter } from '../filters/logic';
/** Stable key for storing per-view options. */
export function viewKey(spec) {
    switch (spec.kind) {
        case 'list':
            return `list:${spec.id}`;
        case 'filter':
            return `filter:${spec.id}`;
        case 'tag':
            return `tag:${spec.name}`;
        default:
            return spec.kind;
    }
}
/** Smart lists group by date; lists and tags keep manual order, ungrouped. */
export function defaultOptions(spec) {
    const smart = spec.kind === 'all' || spec.kind === 'today' || spec.kind === 'week' || spec.kind === 'filter';
    return { groupBy: smart ? 'date' : 'none', sortBy: smart ? 'date' : 'manual', order: 'asc', showCompleted: false, showDetails: true, viewMode: 'list' };
}
const isLive = (t) => t.deletedMs === null;
/** Ids of lists whose tasks appear in All / Today / Next 7 Days. */
function visibleListIds(lists) {
    return new Set(lists.filter((l) => !l.archived).map((l) => l.id));
}
/**
 * The open tasks a view shows (or all of them, done included, when
 * `includeDone`). Smart lists skip archived lists; a list or tag view does not.
 */
export function tasksForView(spec, e, now, includeDone = false) {
    const visible = visibleListIds(e.lists);
    const today = startOfDay(now);
    const wanted = (t) => isLive(t) && (includeDone || t.status === 'open');
    switch (spec.kind) {
        case 'all':
            return e.tasks.filter((t) => wanted(t) && visible.has(t.listId));
        case 'today':
            // Today: due before tomorrow (overdue included), like the server's smart list.
            return e.tasks.filter((t) => wanted(t) && visible.has(t.listId) && t.dueMs !== null && t.dueMs < addDays(today, 1));
        case 'week':
            return e.tasks.filter((t) => wanted(t) && visible.has(t.listId) && t.dueMs !== null && t.dueMs >= today && t.dueMs < addDays(today, 7));
        case 'inbox':
            return e.tasks.filter((t) => wanted(t) && t.listId === e.inboxId);
        case 'list':
            return e.tasks.filter((t) => wanted(t) && t.listId === spec.id);
        case 'tag':
            return e.tasks.filter((t) => wanted(t) && t.tags.includes(spec.name));
        case 'filter': {
            const rule = e.filters.find((f) => f.id === spec.id)?.rule;
            return rule ? e.tasks.filter((t) => wanted(t) && visible.has(t.listId) && matchesFilter(t, rule, dateBucket(t, now), e.assignees[t.id])) : [];
        }
    }
}
const byManual = (a, b) => a.sortOrder - b.sortOrder || cmp(a.id, b.id);
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
/**
 * `asc` is each key's natural direction: soonest date, A to Z, highest
 * priority, oldest, top of the manual order. Undated tasks always sort last
 * by date. Ties fall back to manual order, so the result is deterministic.
 */
export function sortTasks(tasks, by, order) {
    const sign = order === 'asc' ? 1 : -1;
    const key = (a, b) => {
        switch (by) {
            case 'manual':
                return byManual(a, b);
            case 'date': {
                if (a.dueMs === null || b.dueMs === null)
                    return a.dueMs === b.dueMs ? 0 : a.dueMs === null ? 1 * sign : -1 * sign;
                return a.dueMs - b.dueMs;
            }
            case 'title':
                return a.title.localeCompare(b.title, undefined, { sensitivity: 'base', numeric: true });
            case 'priority':
                return b.priority - a.priority;
            case 'created':
                return a.createdMs - b.createdMs;
        }
    };
    return [...tasks].sort((a, b) => sign * key(a, b) || byManual(a, b));
}
const DATE_GROUPS = [
    ['overdue', 'Overdue'],
    ['today', 'Today'],
    ['tomorrow', 'Tomorrow'],
    ['next7', 'Next 7 Days'],
    ['later', 'Later'],
    ['nodate', 'No Date'],
];
export function dateBucket(t, now) {
    if (t.dueMs === null)
        return 'nodate';
    const days = diffDays(now, t.dueMs);
    const overdue = t.isAllDay ? days < 0 : t.dueMs < now && days <= 0;
    if (overdue || days < 0)
        return 'overdue';
    if (days === 0)
        return 'today';
    if (days === 1)
        return 'tomorrow';
    return days < 7 ? 'next7' : 'later';
}
const PRIORITY_GROUPS = [
    [5, 'High Priority'],
    [3, 'Medium Priority'],
    [1, 'Low Priority'],
    [0, 'No Priority'],
];
/**
 * Split (already sorted) tasks into groups, keeping their order within each.
 * Empty groups are omitted. A task with several tags appears under each.
 */
export function groupTasks(tasks, by, ctx) {
    switch (by) {
        case 'none':
            return [{ key: 'all', label: '', tasks }];
        case 'date':
            return DATE_GROUPS.map(([key, label]) => {
                const group = { key, label, tasks: tasks.filter((t) => dateBucket(t, ctx.now) === key) };
                if (key === 'overdue')
                    group.tone = 'overdue';
                return group;
            }).filter((g) => g.tasks.length > 0);
        case 'priority':
            return PRIORITY_GROUPS.map(([p, label]) => ({ key: `p${p}`, label, tasks: tasks.filter((t) => t.priority === p) })).filter((g) => g.tasks.length > 0);
        case 'list': {
            const order = [...ctx.lists].sort((a, b) => a.sortOrder - b.sortOrder);
            return order
                .map((l) => ({ key: l.id, label: l.name, tasks: tasks.filter((t) => t.listId === l.id) }))
                .filter((g) => g.tasks.length > 0);
        }
        case 'tag': {
            const groups = [...ctx.tags]
                .sort((a, b) => a.sortOrder - b.sortOrder)
                .map((tag) => ({ key: tag.name, label: tag.label, tasks: tasks.filter((t) => t.tags.includes(tag.name)) }));
            groups.push({ key: 'notag', label: 'No Tag', tasks: tasks.filter((t) => t.tags.length === 0) });
            return groups.filter((g) => g.tasks.length > 0);
        }
    }
}
/** Gap left between adjacent sort orders, so a drop usually needs no renumbering. */
export const ORDER_STEP = 1024;
/**
 * The sort order for an item dropped between `prev` and `next` (either may be
 * absent at the ends). `null` when the neighbours are adjacent integers, so
 * there is no room and the caller must renumber.
 */
export function sortOrderBetween(prev, next) {
    if (prev === null && next === null)
        return 0;
    if (prev === null)
        return next - ORDER_STEP;
    if (next === null)
        return prev + ORDER_STEP;
    if (next - prev < 2)
        return null;
    return prev + Math.floor((next - prev) / 2);
}
/** Fresh, evenly spaced sort orders for `count` items, for when a gap has closed. */
export function renumber(count) {
    return Array.from({ length: count }, (_, i) => i * ORDER_STEP);
}
/** A new task goes to the top: one step before everything already in the list. */
export function firstSortOrder(existing) {
    return existing.length === 0 ? 0 : Math.min(...existing.map((t) => t.sortOrder)) - ORDER_STEP;
}
/** Checklist progress for a row: `2/3`, or `null` when there is no checklist. */
export function checklistProgress(t) {
    return t.items.length === 0 ? null : `${t.items.filter((i) => i.done).length}/${t.items.length}`;
}
/**
 * What to write when `movedId` is dropped `after` (or before) `targetId`, given
 * `items` in their current order. `null` when nothing would change.
 */
export function reorderItems(items, movedId, targetId, after) {
    if (movedId === targetId)
        return null;
    const moved = items.find((i) => i.id === movedId);
    const rest = items.filter((i) => i.id !== movedId);
    const at = rest.findIndex((i) => i.id === targetId);
    if (!moved || at < 0)
        return null;
    const insert = after ? at + 1 : at;
    const order = sortOrderBetween(rest[insert - 1]?.sortOrder ?? null, rest[insert]?.sortOrder ?? null);
    if (order !== null)
        return order === moved.sortOrder ? null : { kind: 'set', id: movedId, sortOrder: order };
    const next = [...rest.slice(0, insert), moved, ...rest.slice(insert)];
    const fresh = renumber(next.length);
    return { kind: 'renumber', orders: Object.fromEntries(next.map((item, i) => [item.id, fresh[i]])) };
}

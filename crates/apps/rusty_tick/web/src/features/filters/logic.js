import { UNASSIGNED } from '../assignees/logic';
export const DATE_BUCKETS = [
    ['overdue', 'Overdue'],
    ['today', 'Today'],
    ['tomorrow', 'Tomorrow'],
    ['next7', 'Next 7 Days'],
    ['later', 'Later'],
    ['nodate', 'No Date'],
];
export const PRIORITIES = [
    [5, 'High'],
    [3, 'Medium'],
    [1, 'Low'],
    [0, 'None'],
];
export const emptyRule = () => ({ lists: [], tags: [], priorities: [], dates: [], assignees: [] });
/** Whether `task` passes `rule`; `bucket` is its due-date bucket (see `dateBucket`) and `assignee` its assignee name, `''` if none. */
export function matchesFilter(task, rule, bucket, assignee = '') {
    return ((rule.lists.length === 0 || rule.lists.includes(task.listId)) &&
        (rule.tags.length === 0 || task.tags.some((t) => rule.tags.includes(t))) &&
        (rule.priorities.length === 0 || rule.priorities.includes(task.priority)) &&
        (rule.dates.length === 0 || rule.dates.includes(bucket)) &&
        (rule.assignees.length === 0 || rule.assignees.includes(assignee || UNASSIGNED)));
}
const strings = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === 'string') : []);
/** A stored body as a filter body, or `null` if it is not one. Unknown values inside the rule are dropped. */
export function asFilterBody(body) {
    if (typeof body !== 'object' || body === null)
        return null;
    const o = body;
    if (typeof o.name !== 'string' || o.name.trim() === '')
        return null;
    const r = o.rule ?? {};
    const buckets = DATE_BUCKETS.map(([b]) => b);
    const prios = PRIORITIES.map(([p]) => p);
    return {
        name: o.name,
        rule: {
            lists: strings(r.lists),
            tags: strings(r.tags),
            priorities: Array.isArray(r.priorities) ? r.priorities.filter((p) => prios.includes(p)) : [],
            dates: strings(r.dates).filter((d) => buckets.includes(d)),
            assignees: strings(r.assignees),
        },
    };
}

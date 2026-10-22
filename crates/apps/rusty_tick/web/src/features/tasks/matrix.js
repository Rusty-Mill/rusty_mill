import { dropPatch } from './kanban';
import { dateBucket } from './organize';
export const QUADRANTS = [
    { key: 'do', label: 'Do First', hint: 'Important · Urgent', important: true, urgent: true },
    { key: 'plan', label: 'Schedule', hint: 'Important · Not urgent', important: true, urgent: false },
    { key: 'delegate', label: 'Delegate', hint: 'Not important · Urgent', important: false, urgent: true },
    { key: 'drop', label: 'Eliminate', hint: 'Not important · Not urgent', important: false, urgent: false },
];
const isImportant = (t) => t.priority > 0;
const isUrgent = (t, now) => ['overdue', 'today', 'tomorrow'].includes(dateBucket(t, now));
export function quadrantOf(t, now) {
    const imp = isImportant(t);
    const urg = isUrgent(t, now);
    return imp ? (urg ? 'do' : 'plan') : urg ? 'delegate' : 'drop';
}
/** `tasks` split into the four quadrants, in the order of {@link QUADRANTS}; input order is kept. */
export function splitQuadrants(tasks, now) {
    const out = { do: [], plan: [], delegate: [], drop: [] };
    for (const t of tasks)
        out[quadrantOf(t, now)].push(t);
    return out;
}
/**
 * The patch for dropping `task` on quadrant `to`, or `null` if nothing changes.
 * Only the axes that differ are touched: importance sets high priority (or clears it),
 * urgency sets the due date to today (or moves an urgent task into the next 7 days).
 */
export function matrixDropPatch(task, to, now) {
    const want = QUADRANTS.find((q) => q.key === to);
    const patch = {};
    if (want.important !== isImportant(task))
        patch.priority = want.important ? 5 : 0;
    if (want.urgent !== isUrgent(task, now))
        Object.assign(patch, dropPatch(task, 'date', '', want.urgent ? 'today' : 'next7', now));
    return Object.keys(patch).length === 0 ? null : patch;
}

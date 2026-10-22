export const HEADER_HEIGHT = 36;
export const ROW_HEIGHT = 40;
/** Above this many rows the list renders only the visible window. */
export const VIRTUALIZE_ABOVE = 200;
/** A group per header (skipped for the single unlabelled group), its tasks unless collapsed. */
export function buildRows(groups, collapsed, viewKey) {
    const rows = [];
    for (const g of groups) {
        const isCollapsed = !!collapsed[`${viewKey}:${g.key}`];
        if (g.label) {
            const header = { type: 'header', key: g.key, label: g.label, count: g.tasks.length, collapsed: isCollapsed };
            if (g.tone)
                header.tone = g.tone;
            rows.push(header);
        }
        if (!isCollapsed)
            for (const task of g.tasks)
                rows.push({ type: 'task', task, groupKey: g.key });
    }
    return rows;
}
export const rowHeight = (r) => (r.type === 'header' ? HEADER_HEIGHT : ROW_HEIGHT);
/** The rows to draw for a scroll position: those in view plus `overscan` either side. */
export function windowRows(rows, scrollTop, viewport, overscan = 8) {
    const tops = [];
    let total = 0;
    for (const r of rows) {
        tops.push(total);
        total += rowHeight(r);
    }
    if (rows.length === 0)
        return { start: 0, end: 0, offset: 0, total: 0 };
    let first = 0;
    while (first < rows.length - 1 && tops[first] + rowHeight(rows[first]) <= scrollTop)
        first++;
    let last = first;
    while (last < rows.length - 1 && tops[last] < scrollTop + viewport)
        last++;
    const start = Math.max(0, first - overscan);
    const end = Math.min(rows.length, last + 1 + overscan);
    return { start, end, offset: tops[start] ?? 0, total };
}
/** The title shown above a view. */
export function viewTitle(spec, lists, tags, filters, inboxId) {
    switch (spec.kind) {
        case 'all':
            return 'All';
        case 'today':
            return 'Today';
        case 'week':
            return 'Next 7 Days';
        case 'inbox':
            return lists.find((l) => l.id === inboxId)?.name ?? 'Inbox';
        case 'list':
            return lists.find((l) => l.id === spec.id)?.name ?? 'List';
        case 'tag':
            return tags.find((t) => t.name === spec.name)?.label ?? spec.name;
        case 'filter':
            return filters.find((f) => f.id === spec.id)?.name ?? 'Filter';
    }
}

export const MAX_RESULTS = 50;
/** Split `text` around every case-insensitive occurrence of `query`. */
export function highlight(text, query) {
    const q = query.trim().toLowerCase();
    if (!q)
        return [{ text, hit: false }];
    const lower = text.toLowerCase();
    const out = [];
    let at = 0;
    for (let i = lower.indexOf(q); i >= 0; i = lower.indexOf(q, at)) {
        if (i > at)
            out.push({ text: text.slice(at, i), hit: false });
        out.push({ text: text.slice(i, i + q.length), hit: true });
        at = i + q.length;
    }
    if (at < text.length)
        out.push({ text: text.slice(at), hit: false });
    return out;
}
/** A window of `text` around the first match of `query`. */
export function snippet(text, query, radius = 40) {
    const i = text.toLowerCase().indexOf(query.trim().toLowerCase());
    if (i < 0)
        return text.slice(0, radius * 2);
    const start = Math.max(0, i - radius);
    const end = Math.min(text.length, i + query.trim().length + radius);
    return `${start > 0 ? '…' : ''}${text.slice(start, end).replace(/\s+/g, ' ')}${end < text.length ? '…' : ''}`;
}
/**
 * Tasks whose title or notes contain every word of `query` (any order), best
 * first: title matches before notes-only ones, open before completed, then the
 * most recently changed. Trashed tasks never match. An empty query lists the
 * most recently changed tasks.
 */
export function searchTasks(query, tasks, limit = MAX_RESULTS) {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const live = tasks.filter((t) => t.deletedMs === null);
    if (words.length === 0) {
        return [...live].sort((a, b) => b.updatedMs - a.updatedMs).slice(0, 8).map((task) => ({ task, snippet: null }));
    }
    const hits = [];
    for (const task of live) {
        const title = task.title.toLowerCase();
        const body = `${title}\n${task.notes.toLowerCase()}`;
        if (!words.every((w) => body.includes(w)))
            continue;
        const inTitle = words.every((w) => title.includes(w));
        hits.push({ task, inTitle, snippet: inTitle || !task.notes ? null : snippet(task.notes, words.find((w) => task.notes.toLowerCase().includes(w)) ?? words[0]) });
    }
    hits.sort((a, b) => Number(b.inTitle) - Number(a.inTitle) || Number(a.task.status !== 'open') - Number(b.task.status !== 'open') || b.task.updatedMs - a.task.updatedMs);
    return hits.slice(0, limit).map(({ task, snippet: s }) => ({ task, snippet: s }));
}
/** Lists whose name contains `query`, in the sidebar's order. An empty query lists them all. */
export function searchLists(query, lists, limit = MAX_RESULTS) {
    const q = query.trim().toLowerCase();
    return lists
        .filter((l) => !q || l.name.toLowerCase().includes(q))
        .sort((a, b) => a.sortOrder - b.sortOrder)
        .slice(0, limit);
}

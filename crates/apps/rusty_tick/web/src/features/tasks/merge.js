const norm = (text) => text.trim().toLowerCase();
/**
 * Tasks `duplicate` may be merged into: live tasks other than itself and its
 * subtasks (trashing it trashes them, so one kept would go too). With no
 * `query`, those with its title (the likely duplicates); else those whose
 * title contains `query`, its title first. At most `limit`.
 */
export function mergeTargets(tasks, duplicate, query, limit = 50) {
    const title = norm(duplicate.title);
    const q = norm(query);
    const matches = (t) => (q === '' ? norm(t.title) === title : norm(t.title).includes(q));
    return tasks
        .filter((t) => t.id !== duplicate.id && t.parentId !== duplicate.id && t.deletedMs === null && matches(t))
        .sort((a, b) => Number(norm(b.title) === title) - Number(norm(a.title) === title))
        .slice(0, limit);
}

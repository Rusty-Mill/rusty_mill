const KIND = 'import_alias';
const asAlias = (body) => {
    const to = typeof body === 'object' && body !== null ? body.to : undefined;
    return typeof to === 'string' ? { to } : null;
};
/** Merged-away task id -> the id of the task kept in its place. */
export async function readAliases(api) {
    const docs = await api.listDocs(KIND);
    return new Map(docs.flatMap((d) => {
        const alias = asAlias(d.body);
        return alias ? [[d.id, alias.to]] : [];
    }));
}
/**
 * Record that `duplicateId` is now `keepId`: aliases that named the duplicate
 * follow it, and one that named the kept task away is dropped (it is live
 * again), so aliases never chain or loop. The caller trashes the duplicate.
 */
export async function recordMerge(api, duplicateId, keepId) {
    const aliases = await readAliases(api);
    for (const [from, to] of aliases)
        if (to === duplicateId)
            await api.putDoc(KIND, from, { to: keepId });
    if (aliases.has(keepId))
        await api.deleteDoc(KIND, keepId);
    await api.putDoc(KIND, duplicateId, { to: keepId });
}

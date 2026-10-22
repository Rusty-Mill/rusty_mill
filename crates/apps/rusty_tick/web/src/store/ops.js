/** Run `op` against `api`. Resolves with the server's record where there is one. */
export async function runOp(api, op, etagOf = () => undefined) {
    const etag = etagOf(op);
    switch (op.kind) {
        case 'createList':
            return api.createList(op.input);
        case 'updateList':
            return api.updateList(op.id, op.patch, etag);
        case 'deleteList':
            return api.deleteList(op.id);
        case 'createTask':
            return api.createTask(op.input);
        case 'updateTask':
            return api.updateTask(op.id, op.patch, etag);
        case 'trashTask':
            return api.trashTask(op.id, etag);
        case 'restoreTask':
            return api.restoreTask(op.id);
        case 'purgeTask':
            return api.purgeTask(op.id);
        case 'emptyTrash':
            return api.emptyTrash();
        case 'createTag':
            return api.createTag(op.label, op.color);
        case 'updateTag':
            return api.updateTag(op.name, op.patch, etag);
        case 'renameTag':
            return api.renameTag(op.name, op.label);
        case 'deleteTag':
            return api.deleteTag(op.name);
    }
}
/** Creates that a retry may find already done: the server has it, we lost the reply. */
export const isCreate = (op) => op.kind === 'createList' || op.kind === 'createTask' || op.kind === 'createTag';

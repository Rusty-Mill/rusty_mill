import { useMemo } from 'react';
import { useParams } from 'react-router-dom';
import { useData } from './services';
import { parseView } from './paths';
/** The view the current route names, and the open task, if any. */
export function useView() {
    const params = useParams();
    const inboxId = useData((s) => s.inboxId);
    return useMemo(() => ({ spec: parseView(params, inboxId), taskId: params.taskId ?? null }), [params.smart, params.listId, params.tag, params.filterId, params.taskId, inboxId]);
}

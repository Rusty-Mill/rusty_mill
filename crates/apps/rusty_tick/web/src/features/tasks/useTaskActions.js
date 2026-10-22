import { useMemo } from 'react';
import { useActions } from '@/app/services';
import { newId } from '@/lib/id';
import { ORDER_STEP } from './organize';
/** Task operations shared by the row menu, the detail pane and the keyboard shortcuts. */
export function useTaskActions() {
    const actions = useActions();
    return useMemo(() => {
        const fail = (e) => actions.notify('error', e instanceof Error ? e.message : String(e));
        return {
            toggle: (id) => void actions.toggleDone(id).catch(fail),
            setStatus: (id, status) => void actions.updateTask(id, { status }).catch(fail),
            setPriority: (id, priority) => void actions.updateTask(id, { priority }).catch(fail),
            moveTo: (id, listId) => void actions.moveTask(id, listId).catch(fail),
            /** Move to the Trash. */
            remove: (id) => void actions.trashTask(id).then(() => actions.notify('info', 'Moved to Trash')).catch(fail),
            restore: (id) => void actions.restoreTask(id).catch(fail),
            purge: (id) => void actions.purgeTask(id).catch(fail),
            /** A copy just above the original, with fresh ids for its checklist. */
            duplicate: async (t) => {
                try {
                    return await actions.createTask({
                        listId: t.listId,
                        title: t.title,
                        notes: t.notes,
                        kind: t.kind,
                        priority: t.priority,
                        startMs: t.startMs,
                        dueMs: t.dueMs,
                        isAllDay: t.isAllDay,
                        timeZone: t.timeZone,
                        reminders: t.reminders,
                        repeatFlag: t.repeatFlag,
                        tags: t.tags,
                        items: t.items.map((i) => ({ ...i, id: newId(), done: false })),
                        sortOrder: t.sortOrder - Math.floor(ORDER_STEP / 2),
                    });
                }
                catch (e) {
                    fail(e);
                    return null;
                }
            },
        };
    }, [actions]);
}

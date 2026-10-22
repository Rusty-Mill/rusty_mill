import { parseIcs, storedExDates } from '@/lib/ics';
import { itemKey, planSync, sourceFields } from './logic';
import { useSubscriptions } from './store';
/** Fetch the feed and bring the subscription's tasks in line with it. Throws if the fetch fails; nothing changes then. */
export async function syncSubscription(sub, { api, actions, tasks }) {
    const feed = parseIcs(await api.fetchIcs(sub.url)).tasks;
    const plan = planSync(feed, sub.items, tasks);
    const items = { ...plan.items };
    for (const event of plan.create) {
        const { id } = await actions.createTask({ listId: sub.listId, ...sourceFields(event) });
        items[itemKey(event)] = id;
        if (event.exDates.length > 0)
            await actions.updateTask(id, { exDates: storedExDates(event.exDates) }); // a create cannot carry them
    }
    for (const { taskId, patch } of plan.update)
        await actions.updateTask(taskId, patch);
    for (const taskId of plan.trash)
        await actions.trashTask(taskId);
    const { id: _id, ...body } = sub;
    useSubscriptions.getState().put(sub.id, { ...body, items, syncedMs: Date.now() });
    return { created: plan.create.length, updated: plan.update.length, trashed: plan.trash.length };
}
/** Create the list and the subscription, then sync once. If that first sync fails, both are undone. */
export async function subscribe(name, url, deps) {
    const list = await deps.actions.createList({ name });
    const store = useSubscriptions.getState();
    const id = store.add({ name, url, listId: list.id, items: {}, syncedMs: 0 });
    try {
        return await syncSubscription({ id, name, url, listId: list.id, items: {}, syncedMs: 0 }, deps);
    }
    catch (error) {
        useSubscriptions.getState().remove(id);
        await deps.actions.deleteList(list.id);
        throw error;
    }
}
/** Stop following a feed: its list, and so its tasks, go too. */
export async function unsubscribe(sub, actions) {
    useSubscriptions.getState().remove(sub.id);
    await actions.deleteList(sub.listId);
}

/**
 * A module-level store for one kind of client document (`filter`, `countdown`,
 * ...): list on load, optimistic put/remove, a failed save is reported and the
 * change stays on screen. Items are the parsed body plus the doc id.
 */
import { create } from 'zustand';
import { NotFoundError } from '@/api/errors';
import { newId } from '@/lib/id';
/** `parse` returns `null` for a body that is not this kind's (it is skipped). `what` names the thing in messages. */
export function createDocStore(kind, parse, what) {
    let api = null;
    let notify = () => undefined;
    const useDocs = create()((set, get) => ({
        items: [],
        async load(client, notifyFn) {
            api = client;
            notify = notifyFn;
            try {
                const docs = await client.listDocs(kind);
                set({ items: docs.flatMap((d) => { const body = parse(d.body); return body ? [{ id: d.id, ...body }] : []; }) });
            }
            catch {
                /* offline: keep what is shown */
            }
        },
        put(id, body) {
            const item = { id, ...body };
            const items = get().items;
            set({ items: items.some((i) => i.id === id) ? items.map((i) => (i.id === id ? item : i)) : [...items, item] });
            api?.putDoc(kind, id, body).catch(() => notify('error', `Could not save the ${what}.`));
        },
        add(body) {
            const id = newId();
            get().put(id, body);
            return id;
        },
        remove(id) {
            set({ items: get().items.filter((i) => i.id !== id) });
            api?.deleteDoc(kind, id).catch((e) => {
                if (!(e instanceof NotFoundError))
                    notify('error', `Could not delete the ${what}.`);
            });
        },
    }));
    /** For tests: forget everything held at module level. */
    const reset = () => {
        api = null;
        notify = () => undefined;
        useDocs.setState({ items: [] });
    };
    return { useDocs, reset };
}

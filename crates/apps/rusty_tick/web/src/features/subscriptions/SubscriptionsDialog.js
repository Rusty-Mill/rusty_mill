import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { RefreshCw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useActions, useData, useServices } from '@/app/services';
import { Dialog } from '@/components/Dialog';
import { syncSubscription, subscribe, unsubscribe } from './sync';
import { useSubscriptions } from './store';
const summary = (r) => `${r.created} added, ${r.updated} updated, ${r.trashed} removed`;
/** Subscribe to calendar feeds by URL; each lives in its own list and refreshes on demand. */
export function SubscriptionsDialog({ open, onClose }) {
    const { api } = useServices();
    const actions = useActions();
    const tasks = useData((s) => s.tasks);
    const items = useSubscriptions((s) => s.items);
    const [name, setName] = useState('');
    const [url, setUrl] = useState('');
    const [busy, setBusy] = useState(null);
    const [error, setError] = useState('');
    const valid = name.trim() !== '' && url.trim() !== '';
    const run = async (label, job) => {
        setBusy(label);
        setError('');
        try {
            const result = await job();
            if (result)
                actions.notify('info', `${label}: ${summary(result)}`);
        }
        catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
        finally {
            setBusy(null);
        }
    };
    const add = (e) => {
        e.preventDefault();
        if (!valid || busy)
            return;
        void run(name.trim(), async () => {
            const result = await subscribe(name.trim(), url.trim(), { api, actions, tasks });
            setName('');
            setUrl('');
            return result;
        });
    };
    const refresh = (s) => void run(s.name, () => syncSubscription(s, { api, actions, tasks }));
    return (_jsx(Dialog, { open: open, onClose: onClose, title: "Calendar subscriptions", width: 520, children: _jsxs("div", { className: "flex flex-col gap-4 px-6 pb-6 pt-2", children: [items.length === 0 ? (_jsx("p", { className: "text-grey", children: "No subscriptions yet. Paste an https:// or webcal:// calendar link below." })) : (_jsx("ul", { "aria-label": "Subscriptions", className: "flex flex-col gap-2", children: items.map((s) => (_jsxs("li", { className: "flex items-center gap-2 rounded-row border border-line px-3 py-2", children: [_jsxs("div", { className: "min-w-0 flex-1", children: [_jsx("div", { className: "truncate font-semibold", children: s.name }), _jsx("div", { className: "truncate text-s text-grey", children: s.syncedMs ? `Updated ${new Date(s.syncedMs).toLocaleString()}` : 'Not synced yet' })] }), _jsx("button", { type: "button", "aria-label": `Refresh ${s.name}`, disabled: busy !== null, onClick: () => refresh(s), className: "rounded-row p-1.5 text-grey hover:bg-hover disabled:opacity-40", children: _jsx(RefreshCw, { size: 16, className: busy === s.name ? 'animate-spin' : '' }) }), _jsx("button", { type: "button", "aria-label": `Unsubscribe from ${s.name}`, title: "Removes the subscription and its list", disabled: busy !== null, onClick: () => void run(s.name, () => unsubscribe(s, actions)), className: "rounded-row p-1.5 text-grey hover:bg-hover disabled:opacity-40", children: _jsx(Trash2, { size: 16 }) })] }, s.id))) })), _jsxs("form", { onSubmit: add, className: "flex flex-col gap-3 border-t border-line pt-4", children: [_jsxs("label", { className: "flex flex-col gap-1.5", children: [_jsx("span", { className: "text-s text-grey", children: "Name" }), _jsx("input", { "data-autofocus": true, value: name, maxLength: 200, onChange: (e) => setName(e.target.value), placeholder: "Team calendar", className: "h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), _jsxs("label", { className: "flex flex-col gap-1.5", children: [_jsx("span", { className: "text-s text-grey", children: "Calendar link" }), _jsx("input", { value: url, onChange: (e) => setUrl(e.target.value), placeholder: "https://example.com/calendar.ics", inputMode: "url", className: "h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), error && _jsx("p", { role: "alert", className: "text-danger", children: error }), _jsxs("div", { className: "flex items-center justify-end gap-2", children: [busy && _jsxs("span", { className: "mr-auto text-s text-grey", children: ["Fetching ", busy, "\u2026"] }), _jsx("button", { type: "button", onClick: onClose, className: "h-8 rounded-row px-4 hover:bg-hover", children: "Close" }), _jsx("button", { type: "submit", disabled: !valid || busy !== null, className: "h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40", children: "Subscribe" })] })] })] }) }));
}

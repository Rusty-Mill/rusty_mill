import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo, useState } from 'react';
import { useActions, useData, useServices } from '@/app/services';
import { Dialog } from '@/components/Dialog';
import { formatDue } from '@/lib/date';
import { recordMerge } from '../import/aliases';
import { mergeTargets } from './merge';
/**
 * Merge a duplicate into the task to keep: the kept task stays as it is, the
 * duplicate goes to the Trash (so it can be restored), and an import that
 * would add the duplicate again updates the kept task instead.
 */
export function MergeDialog({ task, onClose }) {
    return (_jsx(Dialog, { open: task !== null, onClose: onClose, title: "Merge into\u2026", width: 480, children: task && _jsx(MergeBody, { task: task, onDone: onClose }) }));
}
function MergeBody({ task, onDone }) {
    const { api } = useServices();
    const actions = useActions();
    const taskMap = useData((s) => s.tasks);
    const listMap = useData((s) => s.lists);
    const [query, setQuery] = useState('');
    const now = Date.now();
    const targets = useMemo(() => mergeTargets(Object.values(taskMap), task, query), [taskMap, task, query]);
    const merge = async (keep) => {
        onDone();
        try {
            await recordMerge(api, task.id, keep.id);
            await actions.trashTask(task.id);
            actions.notify('info', `Merged into “${keep.title}”; the duplicate is in the Trash`);
        }
        catch (e) {
            actions.notify('error', `Could not merge: ${e instanceof Error ? e.message : String(e)}`);
        }
    };
    return (_jsxs("div", { className: "flex flex-col gap-3", children: [_jsxs("p", { className: "text-s text-grey", children: ["Pick the task to keep. It stays as it is; \u201C", task.title, "\u201D goes to the Trash, and an import that would add it again updates the kept task instead."] }), _jsx("input", { "data-autofocus": true, "aria-label": "Find the task to keep", autoComplete: "off", value: query, onChange: (e) => setQuery(e.target.value), placeholder: "Search tasks", className: "h-9 rounded-row border border-line bg-transparent px-3 outline-hidden focus:border-primary" }), targets.length === 0 ? (_jsx("p", { className: "py-4 text-center text-s text-grey", children: query ? 'No task matches.' : 'No other task has this title. Search for the one to keep.' })) : (_jsx("ul", { "aria-label": "Tasks to keep", className: "max-h-72 overflow-y-auto", children: targets.map((t) => (_jsx("li", { children: _jsxs("button", { type: "button", onClick: () => void merge(t), className: "flex h-10 w-full items-center gap-2 rounded-row px-2 text-left hover:bg-hover", children: [_jsx("span", { className: "min-w-0 flex-1 truncate", children: t.title }), _jsx("span", { className: "shrink-0 text-s text-grey", children: [listMap[t.listId]?.name, formatDue(t, now)?.text].filter(Boolean).join(' · ') })] }) }, t.id))) }))] }));
}

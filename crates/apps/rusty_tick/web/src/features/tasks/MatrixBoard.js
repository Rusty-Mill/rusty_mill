import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { useActions } from '@/app/services';
import { QUADRANTS, matrixDropPatch, splitQuadrants } from './matrix';
import { TaskCard } from './TaskCard';
/** Eisenhower matrix: four quadrants; dragging a card to one edits its priority and/or due date. */
export function MatrixBoard({ spec, data, selectedId }) {
    const actions = useActions();
    const quadrants = splitQuadrants(data.tasks, data.now);
    const [over, setOver] = useState(null);
    const onDrop = (to, e) => {
        e.preventDefault();
        setOver(null);
        const task = data.tasks.find((t) => t.id === e.dataTransfer.getData('application/x-tick-card').split('|')[0]);
        const patch = task && matrixDropPatch(task, to, data.now);
        if (task && patch)
            void actions.updateTask(task.id, patch).catch((err) => actions.notify('error', err.message));
    };
    return (_jsx("div", { className: "grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-y-auto px-4 pb-4 md:grid-cols-2 md:grid-rows-2", role: "list", "aria-label": "Eisenhower Matrix", children: QUADRANTS.map((q) => (_jsxs("section", { role: "listitem", "aria-label": q.label, onDragOver: (e) => {
                if (e.dataTransfer.types.includes('application/x-tick-card')) {
                    e.preventDefault();
                    setOver(q.key);
                }
            }, onDragLeave: () => setOver((o) => (o === q.key ? null : o)), onDrop: (e) => onDrop(q.key, e), className: `flex min-h-40 flex-col rounded-menu bg-black/[.03] ${over === q.key ? 'ring-2 ring-primary/40' : ''}`, children: [_jsxs("h2", { className: "px-3 py-2 text-base font-semibold", children: [q.label, " ", _jsx("span", { className: "font-normal text-grey", children: quadrants[q.key].length }), _jsx("span", { className: "block text-s font-normal text-grey", children: q.hint })] }), _jsx("ul", { className: "scroll-thin flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2", children: quadrants[q.key].map((t) => (_jsx(TaskCard, { task: t, group: q.key, spec: spec, now: data.now, selected: t.id === selectedId }, t.id))) })] }, q.key))) }));
}

import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useData } from '@/app/services';
import { Dialog } from '@/components/Dialog';
import { computeStats } from './stats';
import { usePrefs } from './prefs';
function Figure({ label, value }) {
    return (_jsxs("div", { className: "rounded-row border border-line px-3 py-2.5", children: [_jsx("dd", { className: "text-h1 font-semibold", children: value }), _jsx("dt", { className: "text-s text-grey", children: label })] }));
}
/** Real numbers from the tasks in memory: nothing is sent anywhere. */
export function StatsDialog({ open, onClose }) {
    const tasks = useData((s) => s.tasks);
    const lists = useData((s) => s.lists);
    const weekStart = usePrefs((s) => s.prefs.weekStart);
    // Recomputed each time the dialog opens, so "today" is right after midnight too.
    const stats = useMemo(() => (open ? computeStats(Object.values(tasks), Object.values(lists), Date.now(), weekStart) : null), [open, tasks, lists, weekStart]);
    return (_jsx(Dialog, { open: open, onClose: onClose, title: "Statistics", width: 480, children: stats && (_jsxs("div", { className: "max-h-[70vh] overflow-y-auto px-6 pb-6", children: [_jsxs("dl", { className: "grid grid-cols-3 gap-3", children: [_jsx(Figure, { label: "Completed today", value: stats.completedToday }), _jsx(Figure, { label: "Completed this week", value: stats.completedThisWeek }), _jsx(Figure, { label: "Completed all time", value: stats.completedAllTime }), _jsx(Figure, { label: "Open tasks", value: stats.open }), _jsx(Figure, { label: "Overdue", value: stats.overdue }), _jsx(Figure, { label: "Day streak", value: stats.streak })] }), _jsx("h3", { className: "mb-1 mt-5 text-base font-semibold", children: "Top lists by completed tasks" }), stats.topLists.length === 0 ? (_jsx("p", { className: "text-base text-grey", children: "No completed tasks yet." })) : (_jsx("ol", { className: "flex flex-col", children: stats.topLists.map((l) => (_jsxs("li", { className: "flex items-center justify-between border-b border-line py-2 last:border-b-0", children: [_jsx("span", { className: "truncate", children: l.name }), _jsx("span", { className: "text-grey", children: l.count })] }, l.listId))) }))] })) }));
}

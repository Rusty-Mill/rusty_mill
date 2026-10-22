import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Trash2 } from 'lucide-react';
import { FocusArt } from '@/components/Illustrations';
import { formatTime } from '@/lib/date';
import { formatDuration, groupByDay } from './logic';
export function FocusRecords({ records, now, hour12, onDelete }) {
    if (records.length === 0) {
        return (_jsxs("div", { className: "flex flex-1 flex-col items-center justify-center gap-2 pb-10 text-grey", children: [_jsx(FocusArt, {}), _jsx("p", { children: "No focus record yet" })] }));
    }
    return (_jsx("ul", { className: "flex-1 overflow-y-auto px-5 pb-4", "aria-label": "Focus records", children: groupByDay(records, now).map((g) => (_jsxs("li", { className: "mt-3", children: [_jsx("h4", { className: "mb-1 text-s font-semibold text-grey", children: g.label }), _jsx("ul", { children: g.records.map((r) => (_jsxs("li", { className: "group flex items-center gap-3 rounded-row py-2 pl-2 pr-1 hover:bg-hover", children: [_jsx("span", { "aria-hidden": true, className: `h-2 w-2 shrink-0 rounded-full ${r.kind === 'pomo' ? 'bg-primary' : 'bg-green'}` }), _jsxs("div", { className: "min-w-0 flex-1", children: [_jsx("div", { className: "truncate", children: r.taskTitle ?? (r.kind === 'pomo' ? 'Focus' : 'Stopwatch') }), _jsxs("div", { className: "text-s text-grey", children: [formatTime(r.startMs, hour12), " \u2013 ", formatTime(r.endMs, hour12), r.interruptions > 0 && ` · ${r.interruptions} ${r.interruptions === 1 ? 'interruption' : 'interruptions'}`] })] }), _jsx("span", { className: "tabular-nums text-grey", children: formatDuration(r.durationSec) }), _jsx("button", { type: "button", "aria-label": `Delete record at ${formatTime(r.startMs, hour12)}`, onClick: () => onDelete(r.id), className: "rounded-row p-1.5 text-grey opacity-0 hover:bg-selected focus-visible:opacity-100 group-hover:opacity-100", children: _jsx(Trash2, { size: 16, strokeWidth: 1.5 }) })] }, r.id))) })] }, g.key))) }));
}

import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { taskPath } from '@/app/paths';
import { TaskCheck } from '@/components/TaskCheck';
import { wash } from '../calendar/EventBar';
import { DEFAULT_COLOR, toEvent } from '../calendar/layout';
import { useTaskActions } from './useTaskActions';
import { addDays, diffDays, startOfDay } from '@/lib/date';
const DAYS = 28;
const COL = 32;
const LABEL = 220;
/** Tasks as bars on a day axis: from their start (or due) day to their due day. Undated tasks are listed without a bar. */
export function TimelineView({ spec, data, selectedId }) {
    const navigate = useNavigate();
    const taskActions = useTaskActions();
    const today = startOfDay(data.now);
    const [from, setFrom] = useState(() => addDays(today, -3));
    const days = Array.from({ length: DAYS }, (_, i) => addDays(from, i));
    const colorOf = (listId) => data.lists.find((l) => l.id === listId)?.color ?? DEFAULT_COLOR;
    if (data.tasks.length === 0)
        return _jsx("div", { className: "flex flex-1 items-center justify-center text-grey", children: "No tasks" });
    const rows = [...data.tasks].sort((a, b) => Number(a.dueMs === null) - Number(b.dueMs === null));
    return (_jsxs("div", { className: "flex min-h-0 flex-1 flex-col", role: "region", "aria-label": "Timeline", children: [_jsxs("div", { className: "flex items-center gap-1 px-4 pb-2", children: [_jsx("button", { type: "button", "aria-label": "Previous weeks", onClick: () => setFrom((f) => addDays(f, -7)), className: "flex h-7 w-7 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(ChevronLeft, { size: 18 }) }), _jsx("button", { type: "button", onClick: () => setFrom(addDays(today, -3)), className: "h-7 rounded-row border border-line px-3 hover:bg-hover", children: "Today" }), _jsx("button", { type: "button", "aria-label": "Next weeks", onClick: () => setFrom((f) => addDays(f, 7)), className: "flex h-7 w-7 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(ChevronRight, { size: 18 }) })] }), _jsx("div", { className: "scroll-thin min-h-0 flex-1 overflow-auto px-4 pb-4", children: _jsxs("div", { style: { width: LABEL + DAYS * COL }, children: [_jsxs("div", { className: "sticky top-0 z-10 flex bg-surface", children: [_jsx("div", { style: { width: LABEL }, className: "shrink-0" }), _jsx("div", { className: "grid", style: { gridTemplateColumns: `repeat(${DAYS}, ${COL}px)` }, children: days.map((d) => {
                                        const dt = new Date(d);
                                        return (_jsxs("div", { className: `flex flex-col items-center py-1 text-s ${d === today ? 'font-semibold text-primary' : 'text-grey'}`, children: [_jsx("span", { children: dt.getDate() === 1 || d === from ? dt.toLocaleDateString('en-US', { month: 'short' }) : dt.toLocaleDateString('en-US', { weekday: 'narrow' }) }), _jsx("span", { children: dt.getDate() })] }, d));
                                    }) })] }), _jsx("ul", { children: rows.map((t) => {
                                const e = toEvent(t);
                                const start = e ? diffDays(from, e.startDay) : 0;
                                const end = e ? diffDays(from, e.endDay) : 0;
                                const visible = e !== null && end >= 0 && start < DAYS;
                                const color = colorOf(t.listId);
                                return (_jsxs("li", { "aria-current": t.id === selectedId ? 'true' : undefined, className: `flex h-9 items-center border-t border-line ${t.id === selectedId ? 'bg-selected' : ''}`, children: [_jsxs("div", { style: { width: LABEL }, className: "flex shrink-0 items-center gap-2 pr-2", children: [_jsx(TaskCheck, { checked: false, priority: t.priority, label: `Complete: ${t.title}`, onChange: () => taskActions.toggle(t.id), size: 16 }), _jsx("button", { type: "button", onClick: () => navigate(taskPath(spec, t.id)), className: "min-w-0 flex-1 truncate text-left", children: t.title })] }), _jsxs("div", { className: "relative grid h-full items-center", style: { gridTemplateColumns: `repeat(${DAYS}, ${COL}px)`, width: DAYS * COL }, children: [visible && (_jsx("button", { type: "button", "aria-label": `${t.title}, bar`, onClick: () => navigate(taskPath(spec, t.id)), className: "mx-0.5 h-6 truncate rounded-full px-2 text-left text-s", style: { gridColumn: `${Math.max(start, 0) + 1} / ${Math.min(end, DAYS - 1) + 2}`, backgroundColor: wash(color, 40), color: 'inherit' }, children: t.title })), days.includes(today) && _jsx("span", { "aria-hidden": true, className: "pointer-events-none absolute inset-y-0 w-px bg-primary/40", style: { left: diffDays(from, today) * COL + COL / 2 } })] })] }, t.id));
                            }) })] }) })] }));
}

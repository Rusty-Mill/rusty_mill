import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { diffDays, formatTime, startOfDay } from '@/lib/date';
import { wash } from './EventBar';
import { agendaGroups, overdueEvents, shortDate } from './layout';
/** When in the day a row happens: the start time, or `All day`. */
function timeText(e, day, hour12) {
    if (e.kind === 'timed')
        return formatTime(e.startMs, hour12);
    if (e.startDay === e.endDay)
        return 'All day';
    return e.startDay === day ? 'Starts' : e.endDay === day ? 'Ends' : 'All day';
}
function Row({ e, day, ctx, listName, first }) {
    const color = ctx.colorOf(e.task);
    const when = day < 0 ? shortDate(e.endDay) : timeText(e, day, ctx.hour12);
    return (_jsxs("li", { className: "grid grid-cols-[84px_28px_1fr] items-stretch gap-x-1 py-1.5", children: [_jsx("span", { className: "pt-2 text-right text-s text-grey", children: when }), _jsxs("span", { "aria-hidden": true, className: "relative flex justify-center", children: [_jsx("span", { className: "absolute inset-y-[-6px] w-px bg-line", style: first ? { top: 12 } : undefined }), _jsx("span", { className: "relative mt-3 h-2.5 w-2.5 rounded-full border-[1.5px] bg-surface", style: { borderColor: color } })] }), _jsxs("button", { type: "button", onClick: (ev) => ctx.openTask(e.task, ev.currentTarget), className: "flex min-w-0 flex-col rounded-row border-l-[3px] px-3 py-2 text-left outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary", style: { backgroundColor: wash(color, 10), borderLeftColor: color }, children: [_jsx("span", { className: "text-s text-primary", children: e.kind === 'timed' ? `${formatTime(e.startMs, ctx.hour12)} - ${formatTime(e.endMs, ctx.hour12)}` : e.startDay === e.endDay ? 'All day' : `${shortDate(e.startDay)} - ${shortDate(e.endDay)}` }), _jsx("span", { className: `truncate ${e.done ? 'text-grey line-through' : 'font-medium'}`, children: e.task.title }), _jsx("span", { className: "sr-only", children: listName(e.task.listId) })] })] }));
}
/** Upcoming tasks grouped by day, days with none left out; overdue ones first. */
export function AgendaView({ range, events, ctx, listName }) {
    const groups = agendaGroups(events, range);
    const overdue = overdueEvents(events, startOfDay(ctx.now));
    if (groups.length === 0 && overdue.length === 0)
        return _jsx("div", { className: "flex flex-1 items-center justify-center text-grey", children: "No tasks in this period" });
    return (_jsxs("div", { className: "min-h-0 flex-1 overflow-y-auto", role: "list", "aria-label": "Agenda", children: [overdue.length > 0 && (_jsxs("section", { role: "listitem", "aria-label": "Overdue", children: [_jsx("h2", { className: "sticky top-0 z-10 border-b border-line bg-surface px-4 py-2 font-semibold text-danger", children: "Overdue" }), _jsx("ul", { children: overdue.map((e) => (_jsx(Row, { e: e, day: -1, ctx: ctx, listName: listName }, e.task.id))) })] })), groups.map(({ day, events: list }) => {
                const rel = diffDays(ctx.now, day);
                const d = new Date(day);
                return (_jsxs("section", { role: "listitem", "aria-label": shortDate(day), className: "grid grid-cols-[92px_1fr] px-6 py-2", children: [_jsxs("h2", { className: "flex items-baseline gap-1.5 pt-3 font-semibold", children: [_jsx("span", { className: `text-title ${rel === 0 ? 'text-primary' : ''}`, children: d.getDate() }), _jsx("span", { className: `text-s font-normal ${rel === 0 ? 'text-primary' : 'text-grey'}`, children: rel === 0 ? 'Today' : rel === 1 ? 'Tomorrow' : d.toLocaleDateString('en-US', { weekday: 'short' }) })] }), _jsx("ul", { children: list.map((e, i) => (_jsx(Row, { e: e, day: day, ctx: ctx, listName: listName, first: i === 0 }, e.task.id))) })] }, day));
            })] }));
}

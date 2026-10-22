import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Popover } from '@/components/Popover';
import { barTime, eventsOnDay, longDate } from './layout';
/** The "+N more" list: everything on one day. */
export function DayPopover({ anchor, day, events, colorOf, hour12, onPick, onClose }) {
    const list = eventsOnDay(events, day);
    return (_jsxs(Popover, { anchor: anchor, open: true, onClose: onClose, role: "dialog", ariaLabel: longDate(day), className: "w-[260px] py-2", children: [_jsx("h2", { className: "px-3 pb-1.5 font-semibold", children: longDate(day) }), _jsx("ul", { className: "max-h-[300px] overflow-y-auto px-1.5", children: list.map((e) => (_jsx("li", { children: _jsxs("button", { type: "button", onClick: () => onPick(e, anchor), className: "flex w-full items-center gap-2 rounded-row px-1.5 py-1 text-left outline-hidden hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary", children: [_jsx("span", { "aria-hidden": true, className: "h-4 w-1 shrink-0 rounded-xs", style: { backgroundColor: colorOf(e) } }), _jsx("span", { className: `min-w-0 flex-1 truncate ${e.done ? 'text-grey line-through' : ''}`, children: e.task.title }), _jsx("span", { className: "shrink-0 text-s text-grey", children: barTime(e, hour12) })] }) }, e.task.id))) })] }));
}

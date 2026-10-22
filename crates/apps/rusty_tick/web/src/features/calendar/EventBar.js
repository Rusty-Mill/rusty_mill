import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { barTime } from './layout';
/** The list colour as a translucent wash; the solid colour goes on the edge. */
export const wash = (color, percent = 18) => `color-mix(in srgb, ${color} ${percent}%, transparent)`;
/** A 20px bar: month cells and the all-day strip. */
export function EventBar({ event, color, hour12, cutStart, cutEnd, onOpen, onDragStart, style }) {
    const time = barTime(event, hour12);
    return (_jsxs("button", { type: "button", draggable: !event.projected, "data-task-id": event.task.id, onClick: (e) => {
            e.stopPropagation();
            onOpen(e.currentTarget);
        }, onDragStart: onDragStart, onDragEnd: () => undefined, style: { backgroundColor: wash(color, event.done ? 12 : 32), borderLeftColor: cutStart ? 'transparent' : color, ...style }, className: `pointer-events-auto flex h-5 w-full min-w-0 items-center gap-1 border-l-[3px] px-1.5 text-left text-s outline-hidden focus-visible:ring-2 focus-visible:ring-primary ${cutStart ? 'rounded-l-none' : 'rounded-l-[4px]'} ${cutEnd ? 'rounded-r-none' : 'rounded-r-[4px]'} ${event.done ? 'text-grey line-through' : 'text-text'} ${event.projected ? 'opacity-70' : ''}`, children: [_jsx("span", { "aria-hidden": true, className: "h-3 w-3 shrink-0 rounded-[3px] border-[1.5px]", style: { borderColor: color, backgroundColor: event.done ? color : 'transparent' } }), _jsx("span", { className: "min-w-0 flex-1 truncate", children: event.task.title }), time && _jsx("span", { className: "shrink-0 text-grey", children: time })] }));
}

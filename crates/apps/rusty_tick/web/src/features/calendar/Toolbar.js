import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { ChevronDown, ChevronLeft, ChevronRight, Plus } from 'lucide-react';
import { useState } from 'react';
import { CALENDAR_MODES } from '@/app/paths';
import { Menu } from '@/components/Menu';
export const MODE_LABEL = { m: 'Month', w: 'Week', d: 'Day', '3': '3 Days', t: '10 Days', a: 'Agenda' };
const NAV = { m: 'month', w: 'week', d: 'day', '3': 'days', t: 'days', a: 'page' };
const btn = 'flex h-8 items-center justify-center rounded-row text-base outline-hidden hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary';
export function Toolbar({ title, mode, showDone, onMode, onShowDone, onPrev, onNext, onToday, onAdd, onImport, onSubscriptions }) {
    const [viewAnchor, setViewAnchor] = useState(null);
    const [open, setOpen] = useState(false);
    return (_jsxs("header", { className: "flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-4 py-2.5", children: [_jsx("h1", { className: "mr-1 min-w-0 text-title font-semibold", "aria-live": "polite", children: title }), _jsxs("div", { className: "ml-auto flex items-center gap-2", children: [_jsx("button", { type: "button", "aria-label": "Add task", onClick: (e) => onAdd(e.currentTarget), className: `${btn} w-8 text-primary`, children: _jsx(Plus, { size: 20, strokeWidth: 1.5 }) }), _jsxs("button", { ref: setViewAnchor, type: "button", "aria-haspopup": "menu", "aria-expanded": open, "aria-label": `View: ${MODE_LABEL[mode]}`, onClick: () => setOpen(true), className: `${btn} gap-1 border border-line pl-3 pr-2`, children: [MODE_LABEL[mode], _jsx(ChevronDown, { size: 16, strokeWidth: 1.5, className: "text-grey" })] }), _jsxs("div", { className: "flex items-center overflow-hidden rounded-row border border-line", children: [_jsx("button", { type: "button", "aria-label": `Previous ${NAV[mode]}`, onClick: onPrev, className: `${btn} w-8 rounded-none text-grey`, children: _jsx(ChevronLeft, { size: 20, strokeWidth: 1.5 }) }), _jsx("button", { type: "button", onClick: onToday, className: `${btn} rounded-none border-x border-line px-3`, children: "Today" }), _jsx("button", { type: "button", "aria-label": `Next ${NAV[mode]}`, onClick: onNext, className: `${btn} w-8 rounded-none text-grey`, children: _jsx(ChevronRight, { size: 20, strokeWidth: 1.5 }) })] })] }), _jsx(Menu, { anchor: viewAnchor, open: open, onClose: () => setOpen(false), placement: "bottom-end", label: "Calendar view", items: [
                    ...CALENDAR_MODES.map((m) => ({ id: m, label: MODE_LABEL[m], checked: m === mode, onSelect: () => onMode(m) })),
                    'separator',
                    { id: 'done', label: 'Show completed', checked: showDone, onSelect: () => onShowDone(!showDone) },
                    { id: 'import', label: 'Import calendar (.ics)', onSelect: onImport },
                    { id: 'subscriptions', label: 'Calendar subscriptions…', onSelect: onSubscriptions },
                ] })] }));
}

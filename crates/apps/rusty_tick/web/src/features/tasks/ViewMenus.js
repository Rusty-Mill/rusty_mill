import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { ArrowUpDown, Columns3, GanttChart, LayoutGrid, List as ListIcon, Printer, SlidersHorizontal, Eye, EyeOff, MoreHorizontal } from 'lucide-react';
import { useRef, useState } from 'react';
import { Menu } from '@/components/Menu';
const GROUP_LABEL = { date: 'Date', list: 'List', priority: 'Priority', tag: 'Tag', none: 'None' };
const SORT_LABEL = { manual: 'Custom', date: 'Date', title: 'Title', priority: 'Priority', created: 'Created' };
const ORDER_LABEL = { asc: 'Ascending', desc: 'Descending' };
/** The sort icon's popover: three rows (Group by, Sort by, Order), each opening a submenu of choices. */
export function SortMenu({ options, onChange }) {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    const pick = (key, labels) => Object.entries(labels).map(([value, label]) => ({ id: value, label, checked: options[key] === value, onSelect: () => onChange({ [key]: value }) }));
    const items = [
        { id: 'group', label: 'Group by', hint: GROUP_LABEL[options.groupBy], submenu: pick('groupBy', GROUP_LABEL) },
        { id: 'sort', label: 'Sort by', hint: SORT_LABEL[options.sortBy], submenu: pick('sortBy', SORT_LABEL) },
        { id: 'order', label: 'Order', hint: ORDER_LABEL[options.order], submenu: pick('order', ORDER_LABEL) },
    ];
    return (_jsxs(_Fragment, { children: [_jsx("button", { ref: ref, type: "button", "aria-label": "Sort", "aria-haspopup": "menu", "aria-expanded": open, onClick: () => setOpen((o) => !o), className: "flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(ArrowUpDown, { size: 18 }) }), _jsx(Menu, { anchor: ref.current, open: open, onClose: () => setOpen(false), items: items, label: "Sort", placement: "bottom-end", className: "min-w-[220px]" })] }));
}
const MODES = [
    { mode: 'list', label: 'List', icon: _jsx(ListIcon, { size: 18 }) },
    { mode: 'kanban', label: 'Kanban', icon: _jsx(Columns3, { size: 18 }) },
    { mode: 'timeline', label: 'Timeline', icon: _jsx(GanttChart, { size: 18 }) },
    { mode: 'matrix', label: 'Eisenhower Matrix', icon: _jsx(LayoutGrid, { size: 18 }) },
];
/** The "..." popover: view switcher, Hide/Show Completed, Show Details, View Options, Print. */
export function MoreMenu({ options, onChange, viewMode, onViewMode, onViewOptions }) {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    const items = [
        {
            id: 'views',
            custom: (_jsx("div", { role: "radiogroup", "aria-label": "View", className: "flex gap-1 px-3 pb-1.5 pt-1", children: MODES.map((m) => (_jsx("button", { type: "button", role: "radio", "aria-checked": viewMode === m.mode, "aria-label": m.label, title: m.label, onClick: () => {
                        onViewMode(m.mode);
                        setOpen(false);
                    }, className: `flex h-9 flex-1 items-center justify-center rounded-row ${viewMode === m.mode ? 'bg-primary/10 text-primary' : 'text-grey hover:bg-hover'}`, children: m.icon }, m.mode))) })),
        },
        'separator',
        { id: 'completed', label: options.showCompleted ? 'Hide Completed' : 'Show Completed', icon: options.showCompleted ? _jsx(EyeOff, { size: 16 }) : _jsx(Eye, { size: 16 }), onSelect: () => onChange({ showCompleted: !options.showCompleted }) },
        { id: 'details', label: 'Show Details', checked: options.showDetails, onSelect: () => onChange({ showDetails: !options.showDetails }) },
        { id: 'options', label: 'View Options', icon: _jsx(SlidersHorizontal, { size: 16 }), onSelect: onViewOptions },
        'separator',
        { id: 'print', label: 'Print', icon: _jsx(Printer, { size: 16 }), onSelect: () => window.print() },
    ];
    return (_jsxs(_Fragment, { children: [_jsx("button", { ref: ref, type: "button", "aria-label": "More", "aria-haspopup": "menu", "aria-expanded": open, onClick: () => setOpen((o) => !o), className: "flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(MoreHorizontal, { size: 18 }) }), _jsx(Menu, { anchor: ref.current, open: open, onClose: () => setOpen(false), items: items, label: "More", placement: "bottom-end", className: "min-w-[220px]" })] }));
}

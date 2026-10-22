import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { CalendarClock, ChevronRight, Copy, Flag, FolderInput, GitMerge, GripVertical, ListChecks, MoreHorizontal, Trash2 } from 'lucide-react';
import { memo, useRef, useState } from 'react';
import { Menu } from '@/components/Menu';
import { PRIORITY_COLOR, PRIORITY_LABEL, TaskCheck } from '@/components/TaskCheck';
import { formatDue } from '@/lib/date';
import { checklistProgress } from './organize';
const TONE_CLASS = { overdue: 'text-danger', today: 'text-primary', future: 'text-grey' };
/** One 40px task row. Memoised: a list of hundreds re-renders only the rows that changed. */
export const TaskRow = memo(function TaskRow({ task, selected, now, hour12, showDetails, showList, list, tagsByName, assignee, lists, actions, dragProps, dropIndicator }) {
    const [menuOpen, setMenuOpen] = useState(false);
    const [at, setAt] = useState(null);
    const moreRef = useRef(null);
    const pointRef = useRef(null);
    const done = task.status !== 'open';
    const due = formatDue(task, now, hour12);
    const progress = checklistProgress(task);
    const menu = [
        { id: 'open', label: 'Open', icon: _jsx(ChevronRight, { size: 16 }), onSelect: () => actions.open(task.id) },
        { id: 'dup', label: 'Duplicate', icon: _jsx(Copy, { size: 16 }), onSelect: () => actions.duplicate(task) },
        {
            id: 'priority',
            label: 'Priority',
            icon: _jsx(Flag, { size: 16 }),
            submenu: [5, 3, 1, 0].map((p) => ({
                id: `p${p}`,
                label: PRIORITY_LABEL[p],
                icon: _jsx(Flag, { size: 16, fill: p ? PRIORITY_COLOR[p] : 'none', color: PRIORITY_COLOR[p] }),
                checked: task.priority === p,
                onSelect: () => actions.setPriority(task.id, p),
            })),
        },
        {
            id: 'move',
            label: 'Move to',
            icon: _jsx(FolderInput, { size: 16 }),
            submenu: lists.filter((l) => !l.archived).map((l) => ({ id: l.id, label: l.name, checked: l.id === task.listId, onSelect: () => actions.moveTo(task.id, l.id) })),
        },
        { id: 'merge', label: 'Merge into…', icon: _jsx(GitMerge, { size: 16 }), onSelect: () => actions.merge(task) },
        'separator',
        { id: 'delete', label: 'Delete', icon: _jsx(Trash2, { size: 16 }), danger: true, onSelect: () => actions.remove(task.id) },
    ];
    const tagChips = showDetails ? task.tags.map((n) => tagsByName[n]).filter((t) => !!t).slice(0, 2) : [];
    return (_jsxs("li", { ...dragProps, "data-task-id": task.id, "aria-current": selected ? 'true' : undefined, onContextMenu: (e) => {
            e.preventDefault();
            setAt({ x: e.clientX, y: e.clientY });
        }, className: `group relative flex h-10 items-center rounded-row border-b border-line/50 pr-2 transition-colors duration-150 ${selected ? 'bg-selected' : 'hover:bg-hover'} ${dropIndicator === 'before' ? 'before:absolute before:-top-px before:left-2 before:right-2 before:z-10 before:h-0.5 before:rounded before:bg-primary' : ''} ${dropIndicator === 'after' ? 'after:absolute after:-bottom-px after:left-2 after:right-2 after:z-10 after:h-0.5 after:rounded after:bg-primary' : ''}`, children: [dragProps && (_jsx("span", { "aria-hidden": true, className: "absolute -left-0.5 hidden cursor-grab text-grey group-hover:block", children: _jsx(GripVertical, { size: 14 }) })), _jsx("div", { className: "flex w-10 shrink-0 items-center justify-center", children: _jsx(TaskCheck, { checked: done, priority: task.priority, label: task.title, onChange: () => actions.toggle(task.id) }) }), _jsxs("button", { type: "button", onClick: () => actions.open(task.id), className: "flex h-full min-w-0 flex-1 items-center gap-2 text-left", children: [_jsx("span", { className: `min-w-0 flex-1 truncate ${done ? 'text-grey line-through' : ''}`, children: task.title }), progress && showDetails && (_jsxs("span", { className: "flex shrink-0 items-center gap-1 text-s text-grey", title: "Checklist progress", children: [_jsx(ListChecks, { size: 13, "aria-hidden": true }), progress] })), tagChips.map((t) => (_jsx("span", { className: "shrink-0 rounded-full px-1.5 text-s", style: { color: t.color ?? 'rgb(var(--grey))', backgroundColor: `${t.color ?? '#a8a8a8'}1a` }, children: t.label }, t.name))), showDetails && assignee && (_jsx("span", { className: "max-w-[90px] shrink-0 truncate rounded-full bg-black/5 px-1.5 text-s text-grey", title: `Assigned to ${assignee}`, children: assignee })), showDetails && showList && list && _jsx("span", { className: "max-w-[110px] shrink-0 truncate text-s text-grey", children: list.name }), due && (_jsxs("span", { className: `flex shrink-0 items-center gap-1 text-s ${done ? 'text-grey' : TONE_CLASS[due.tone]}`, children: [task.repeatFlag && _jsx(CalendarClock, { size: 12, "aria-label": "Repeats" }), due.text] }))] }), _jsx("button", { ref: moreRef, type: "button", "aria-label": `Options for ${task.title}`, "aria-haspopup": "menu", onClick: () => setMenuOpen(true), className: "ml-1 hidden h-6 w-6 shrink-0 items-center justify-center rounded text-grey hover:bg-black/5 group-focus-within:flex group-hover:flex", children: _jsx(MoreHorizontal, { size: 16 }) }), _jsx(Menu, { anchor: moreRef.current, open: menuOpen, onClose: () => setMenuOpen(false), items: menu, label: "Task options", placement: "bottom-end" }), _jsx("span", { ref: pointRef, "aria-hidden": true, style: { position: 'fixed', left: at?.x ?? 0, top: at?.y ?? 0, width: 0, height: 0 } }), _jsx(Menu, { anchor: at ? pointRef.current : null, open: !!at, onClose: () => setAt(null), items: menu, label: "Task options" })] }));
});

import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Menu as MenuIcon } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { taskPath } from '@/app/paths';
import { useActions, useData } from '@/app/services';
import { NoTasksArt } from '@/components/Illustrations';
import { useMedia } from '@/lib/hooks';
import { useReorderDrag } from '@/lib/useReorderDrag';
import { NARROW } from '@/app/Shell';
import { useUi } from '@/store/ui';
import { usePrefs } from '../settings/prefs';
import { KanbanBoard } from './KanbanBoard';
import { MatrixBoard } from './MatrixBoard';
import { TimelineView } from './TimelineView';
import { QuickAdd } from './QuickAdd';
import { MergeDialog } from './MergeDialog';
import { TaskRow } from './TaskRow';
import { reorderItems, viewKey } from './organize';
import { buildRows, windowRows, ROW_HEIGHT, HEADER_HEIGHT, VIRTUALIZE_ABOVE } from './rows';
import { useTaskActions } from './useTaskActions';
import { useViewData } from './useViewData';
import { MoreMenu, SortMenu } from './ViewMenus';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { assigneeMap } from '../assignees/logic';
import { useAssignees } from '../assignees/store';
/** The middle column: title, sort and view menus, the quick-add box, and the grouped rows. */
export function ListColumn({ spec, selectedId }) {
    const navigate = useNavigate();
    const actions = useActions();
    const taskActions = useTaskActions();
    const inboxId = useData((s) => s.inboxId);
    const data = useViewData(spec);
    const { options, groups, lists, tags, now } = data;
    const setOptions = useUi((s) => s.setOptions);
    const collapsedGroups = useUi((s) => s.collapsedGroups);
    const toggleGroup = useUi((s) => s.toggleGroup);
    const toggleSidebar = useUi((s) => s.toggleSidebar);
    const toggleDrawer = useUi((s) => s.toggleDrawer);
    const narrow = useMedia(NARROW);
    const focusQuickAdd = useUi((s) => s.quickAddFocus);
    const hour12 = usePrefs((s) => s.prefs.hour12);
    void focusQuickAdd;
    const key = viewKey(spec);
    const list = spec.kind === 'list' ? lists.find((l) => l.id === spec.id) : undefined;
    const viewMode = list ? list.viewMode : options.viewMode;
    const setViewMode = (mode) => {
        setOptions(spec, { viewMode: mode });
        if (list)
            void actions.updateList(list.id, { viewMode: mode });
    };
    const listById = useMemo(() => Object.fromEntries(lists.map((l) => [l.id, l])), [lists]);
    const assigneeItems = useAssignees((s) => s.items);
    const assignees = useMemo(() => assigneeMap(assigneeItems), [assigneeItems]);
    const tagsByName = useMemo(() => Object.fromEntries(tags.map((t) => [t.name, t])), [tags]);
    const rows = useMemo(() => buildRows(groups, collapsedGroups, key), [groups, collapsedGroups, key]);
    const [merging, setMerging] = useState(null);
    const rowActions = useMemo(() => ({
        open: (id) => navigate(taskPath(spec, id)),
        toggle: taskActions.toggle,
        setPriority: taskActions.setPriority,
        moveTo: taskActions.moveTo,
        duplicate: (t) => void taskActions.duplicate(t),
        merge: setMerging,
        remove: taskActions.remove,
    }), [navigate, spec, taskActions]);
    // Dragging reorders only when the order is the user's own and rows are not split into groups.
    const canDrag = options.sortBy === 'manual' && options.groupBy === 'none' && !options.showCompleted;
    const drag = useReorderDrag('application/x-tick-task', (moved, target, after) => {
        const r = reorderItems(data.tasks, moved, target, after);
        if (!r)
            return;
        if (r.kind === 'set')
            void actions.reorderTask(r.id, r.sortOrder);
        else
            for (const [id, sortOrder] of Object.entries(r.orders))
                void actions.reorderTask(id, sortOrder);
    });
    const total = groups.reduce((n, g) => n + g.tasks.length, 0);
    const showList = spec.kind !== 'list' && spec.kind !== 'inbox';
    return (_jsxs("section", { "aria-label": data.title, className: "flex min-w-0 flex-1 flex-col", children: [_jsxs("header", { className: "flex h-14 shrink-0 items-center gap-1 px-4", children: [_jsx("button", { type: "button", "aria-label": "Toggle sidebar", onClick: narrow ? toggleDrawer : toggleSidebar, className: "flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(MenuIcon, { size: 20 }) }), _jsx("h1", { className: "ml-1 min-w-0 flex-1 truncate text-title font-semibold", children: data.title }), _jsx(SortMenu, { options: options, onChange: (patch) => setOptions(spec, patch) }), _jsx(MoreMenu, { options: options, onChange: (patch) => setOptions(spec, patch), viewMode: viewMode, onViewMode: setViewMode, onViewOptions: () => document.querySelector('button[aria-label="Sort"]')?.click() })] }), _jsx(QuickAdd, { view: spec, lists: lists, inboxId: inboxId, tasks: data.tasks, now: now }), viewMode === 'kanban' ? (_jsx(KanbanBoard, { spec: spec, data: data, selectedId: selectedId })) : viewMode === 'matrix' ? (_jsx(MatrixBoard, { spec: spec, data: data, selectedId: selectedId })) : viewMode === 'timeline' ? (_jsx(TimelineView, { spec: spec, data: data, selectedId: selectedId })) : total === 0 ? (_jsxs("button", { type: "button", onClick: () => useUi.getState().focusQuickAdd(), className: "flex flex-1 flex-col items-center justify-center gap-2 pb-24 text-grey outline-hidden", children: [_jsx(NoTasksArt, {}), _jsx("span", { className: "text-base font-semibold text-text", children: "No tasks" }), _jsx("span", { className: "text-s", children: "Click the input box to add" })] })) : (_jsx(RowList, { rows: rows, renderTask: (r) => {
                    const dropIndicator = drag.indicator?.id === r.task.id ? (drag.indicator.after ? 'after' : 'before') : null;
                    return (_jsx(TaskRow, { task: r.task, selected: r.task.id === selectedId, now: now, hour12: hour12, showDetails: options.showDetails, showList: showList, list: listById[r.task.listId], tagsByName: tagsByName, assignee: assignees[r.task.id], lists: lists, actions: rowActions, dragProps: canDrag && r.groupKey !== 'done' ? drag.bind(r.task.id) : undefined, dropIndicator: dropIndicator }, r.task.id));
                }, onToggleGroup: (g) => toggleGroup(`${key}:${g}`), selectedId: selectedId })), _jsx(MergeDialog, { task: merging, onClose: () => setMerging(null) })] }));
}
/** Draws every row, or only the visible window once there are more than a couple of hundred. */
function RowList({ rows, renderTask, onToggleGroup, selectedId }) {
    const scroller = useRef(null);
    const [scrollTop, setScrollTop] = useState(0);
    const [viewport, setViewport] = useState(800);
    const virtual = rows.length > VIRTUALIZE_ABOVE;
    useEffect(() => {
        const el = scroller.current;
        if (!el || typeof ResizeObserver === 'undefined')
            return;
        const ro = new ResizeObserver(() => setViewport(el.clientHeight));
        ro.observe(el);
        setViewport(el.clientHeight);
        return () => ro.disconnect();
    }, []);
    // Keep the selected task in view when it changes (J/K, or opening one).
    useEffect(() => {
        if (!selectedId || !scroller.current)
            return;
        const el = scroller.current.querySelector(`[data-task-id="${selectedId}"]`);
        el?.scrollIntoView?.({ block: 'nearest' });
    }, [selectedId]);
    const onScroll = useCallback((e) => setScrollTop(e.currentTarget.scrollTop), []);
    const win = virtual ? windowRows(rows, scrollTop, viewport) : { start: 0, end: rows.length, offset: 0, total: 0 };
    const shown = rows.slice(win.start, win.end);
    const body = (_jsx("ul", { role: "list", "aria-label": "Tasks", className: "px-2", style: virtual ? { position: 'absolute', top: win.offset, left: 0, right: 0 } : undefined, children: shown.map((r) => r.type === 'header' ? (_jsx("li", { style: { height: HEADER_HEIGHT }, className: "flex items-center", children: _jsxs("button", { type: "button", "aria-expanded": !r.collapsed, onClick: () => onToggleGroup(r.key), className: "flex items-center gap-1 rounded px-1 text-base font-semibold hover:text-primary", children: [r.collapsed ? _jsx(ChevronRight, { size: 16, className: "text-grey" }) : _jsx(ChevronDown, { size: 16, className: "text-grey" }), _jsx("span", { className: r.tone === 'overdue' ? 'text-danger' : '', children: r.label }), _jsx("span", { className: "ml-1 font-normal text-grey", children: r.count })] }) }, `h:${r.key}`)) : (renderTask(r))) }));
    return (_jsx("div", { ref: scroller, onScroll: onScroll, className: "scroll-thin relative min-h-0 flex-1 overflow-y-auto pb-6", style: { ['--row']: `${ROW_HEIGHT}px` }, children: virtual ? _jsx("div", { style: { height: win.total, position: 'relative' }, children: body }) : body }));
}

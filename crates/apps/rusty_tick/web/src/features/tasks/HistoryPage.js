import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { ChevronDown, Menu as MenuIcon, RotateCcw, Trash2 } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { PATHS } from '@/app/paths';
import { NARROW } from '@/app/Shell';
import { useActions, useData } from '@/app/services';
import { Confirm } from '@/components/Confirm';
import { NoTasksArt } from '@/components/Illustrations';
import { Menu } from '@/components/Menu';
import { TaskCheck } from '@/components/TaskCheck';
import { formatTime } from '@/lib/date';
import { useMedia, useNow } from '@/lib/hooks';
import { useUi } from '@/store/ui';
import { usePrefs } from '../settings/prefs';
import { DetailPane } from './detail/DetailPane';
import { completedGroups, RANGE_LABEL, trashGroups } from './histories';
import { useTaskActions } from './useTaskActions';
function useTasksAndLists() {
    const taskMap = useData((s) => s.tasks);
    const listMap = useData((s) => s.lists);
    const tasks = useMemo(() => Object.values(taskMap), [taskMap]);
    const lists = useMemo(() => Object.values(listMap).sort((a, b) => a.sortOrder - b.sortOrder), [listMap]);
    return { tasks, lists, listMap };
}
function Shell({ title, base, taskId, toolbar, children }) {
    const narrow = useMedia(NARROW);
    const toggleSidebar = useUi((s) => s.toggleSidebar);
    const toggleDrawer = useUi((s) => s.toggleDrawer);
    const paths = useMemo(() => ({ list: base, task: (id) => `${base}/${id}` }), [base]);
    return (_jsxs(_Fragment, { children: [_jsxs("section", { "aria-label": title, className: "flex min-w-0 flex-1 flex-col", children: [_jsxs("header", { className: "flex h-14 shrink-0 items-center gap-2 px-4", children: [_jsx("button", { type: "button", "aria-label": "Toggle sidebar", onClick: narrow ? toggleDrawer : toggleSidebar, className: "flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(MenuIcon, { size: 20 }) }), _jsx("h1", { className: "ml-1 min-w-0 flex-1 truncate text-title font-semibold", children: title }), toolbar] }), children] }), _jsx(DetailPane, { paths: paths, taskId: taskId ?? null })] }));
}
function Groups({ groups, empty, row }) {
    if (groups.length === 0) {
        return (_jsxs("div", { className: "flex flex-1 flex-col items-center justify-center gap-2 pb-24 text-grey", children: [_jsx(NoTasksArt, {}), _jsx("span", { className: "text-base font-semibold text-text", children: empty })] }));
    }
    return (_jsx("div", { className: "scroll-thin min-h-0 flex-1 overflow-y-auto px-2 pb-6", children: groups.map((g) => (_jsxs("section", { "aria-label": g.label, children: [_jsxs("h2", { className: "flex h-9 items-center gap-2 px-2 text-base font-semibold", children: [g.label, _jsx("span", { className: "font-normal text-grey", children: g.tasks.length })] }), _jsx("ul", { role: "list", children: g.tasks.map(row) })] }, g.key))) }));
}
function Dropdown({ label, value, items }) {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    return (_jsxs(_Fragment, { children: [_jsxs("button", { ref: ref, type: "button", "aria-label": label, "aria-haspopup": "menu", "aria-expanded": open, onClick: () => setOpen((o) => !o), className: "flex h-8 items-center gap-1 rounded-row px-2 text-base hover:bg-hover", children: [value, _jsx(ChevronDown, { size: 14, className: "text-grey" })] }), _jsx(Menu, { anchor: ref.current, open: open, onClose: () => setOpen(false), items: items, label: label, placement: "bottom-end" })] }));
}
/** Completed: filters by date and list, grouped by the day each task was finished. */
export function CompletedPage() {
    const { taskId } = useParams();
    const navigate = useNavigate();
    const now = useNow(60_000);
    const weekStart = usePrefs((s) => s.prefs.weekStart);
    const hour12 = usePrefs((s) => s.prefs.hour12);
    const taskActions = useTaskActions();
    const { tasks, lists, listMap } = useTasksAndLists();
    const [filter, setFilter] = useState({ range: 'all', listId: null });
    const groups = useMemo(() => completedGroups(tasks, filter, now, weekStart), [tasks, filter, now, weekStart]);
    const rangeItems = Object.keys(RANGE_LABEL).map((r) => ({ id: r, label: RANGE_LABEL[r], checked: filter.range === r, onSelect: () => setFilter((f) => ({ ...f, range: r })) }));
    const listItems = [
        { id: 'all', label: 'All Lists', checked: filter.listId === null, onSelect: () => setFilter((f) => ({ ...f, listId: null })) },
        ...lists.map((l) => ({ id: l.id, label: l.name, checked: filter.listId === l.id, onSelect: () => setFilter((f) => ({ ...f, listId: l.id })) })),
    ];
    return (_jsx(Shell, { title: "Completed", base: PATHS.completed, taskId: taskId, toolbar: _jsxs(_Fragment, { children: [_jsx(Dropdown, { label: "Date range", value: RANGE_LABEL[filter.range], items: rangeItems }), _jsx(Dropdown, { label: "List", value: filter.listId === null ? 'All Lists' : (listMap[filter.listId]?.name ?? 'All Lists'), items: listItems })] }), children: _jsx(Groups, { groups: groups, empty: "No completed tasks", row: (t) => (_jsxs("li", { "data-task-id": t.id, "aria-current": t.id === taskId ? 'true' : undefined, className: `group flex h-10 items-center rounded-row pr-3 ${t.id === taskId ? 'bg-selected' : 'hover:bg-hover'}`, children: [_jsx("div", { className: "flex w-10 shrink-0 items-center justify-center", children: _jsx(TaskCheck, { checked: true, priority: t.priority, label: `Reopen: ${t.title}`, onChange: () => taskActions.toggle(t.id) }) }), _jsxs("button", { type: "button", onClick: () => navigate(`${PATHS.completed}/${t.id}`), className: "flex h-full min-w-0 flex-1 items-center gap-2 text-left text-grey", children: [_jsx("span", { className: "min-w-0 flex-1 truncate line-through", children: t.title }), _jsx("span", { className: "shrink-0 text-s", children: listMap[t.listId]?.name }), _jsx("span", { className: "shrink-0 text-s", children: t.completedMs !== null && formatTime(t.completedMs, hour12) })] })] }, t.id)) }) }));
}
/** Trash: deleted tasks, each restorable or deletable for good. */
export function TrashPage() {
    const { taskId } = useParams();
    const navigate = useNavigate();
    const actions = useActions();
    const taskActions = useTaskActions();
    const now = useNow(60_000);
    const { tasks, listMap } = useTasksAndLists();
    const groups = useMemo(() => trashGroups(tasks, now), [tasks, now]);
    const total = groups.reduce((n, g) => n + g.tasks.length, 0);
    const [confirmEmpty, setConfirmEmpty] = useState(false);
    const [confirmOne, setConfirmOne] = useState(null);
    return (_jsxs(Shell, { title: "Trash", base: PATHS.trash, taskId: taskId, toolbar: _jsxs("button", { type: "button", disabled: total === 0, onClick: () => setConfirmEmpty(true), className: "flex h-8 items-center gap-1.5 rounded-row px-2 text-base text-danger hover:bg-hover disabled:text-grey disabled:opacity-50", children: [_jsx(Trash2, { size: 16 }), " Empty Trash"] }), children: [_jsx(Groups, { groups: groups, empty: "Trash is empty", row: (t) => (_jsxs("li", { "data-task-id": t.id, "aria-current": t.id === taskId ? 'true' : undefined, className: `group flex h-10 items-center rounded-row pl-4 pr-2 ${t.id === taskId ? 'bg-selected' : 'hover:bg-hover'}`, children: [_jsxs("button", { type: "button", onClick: () => navigate(`${PATHS.trash}/${t.id}`), className: "flex h-full min-w-0 flex-1 items-center gap-2 text-left text-grey", children: [_jsx("span", { className: "min-w-0 flex-1 truncate", children: t.title }), _jsx("span", { className: "shrink-0 text-s", children: listMap[t.listId]?.name ?? 'Deleted list' })] }), _jsx("button", { type: "button", "aria-label": `Restore ${t.title}`, title: "Restore", onClick: () => taskActions.restore(t.id), className: "ml-1 hidden h-7 w-7 items-center justify-center rounded text-grey hover:bg-black/5 hover:text-primary group-focus-within:flex group-hover:flex", children: _jsx(RotateCcw, { size: 15 }) }), _jsx("button", { type: "button", "aria-label": `Delete ${t.title} forever`, title: "Delete forever", onClick: () => setConfirmOne(t), className: "hidden h-7 w-7 items-center justify-center rounded text-grey hover:bg-black/5 hover:text-danger group-focus-within:flex group-hover:flex", children: _jsx(Trash2, { size: 15 }) })] }, t.id)) }), _jsx(Confirm, { open: confirmEmpty, title: "Empty Trash?", message: `${total} task${total === 1 ? '' : 's'} will be deleted permanently. This cannot be undone.`, confirmLabel: "Empty Trash", danger: true, onCancel: () => setConfirmEmpty(false), onConfirm: () => {
                    setConfirmEmpty(false);
                    navigate(PATHS.trash);
                    void actions.emptyTrash().catch((e) => actions.notify('error', e.message));
                } }), _jsx(Confirm, { open: !!confirmOne, title: "Delete forever?", message: `"${confirmOne?.title}" will be deleted permanently. This cannot be undone.`, confirmLabel: "Delete", danger: true, onCancel: () => setConfirmOne(null), onConfirm: () => {
                    const t = confirmOne;
                    setConfirmOne(null);
                    if (!t)
                        return;
                    if (t.id === taskId)
                        navigate(PATHS.trash);
                    taskActions.purge(t.id);
                } })] }));
}

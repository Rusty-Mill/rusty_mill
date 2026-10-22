import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { AlignJustify, Archive, CheckCircle2, ChevronDown, ChevronRight, Crown, FileText, Filter as FilterIcon, Inbox, Layers, MoreHorizontal, Pencil, Plus, Trash2, Undo2 } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { NavLink, useLocation, useNavigate } from 'react-router-dom';
import { PATHS, viewPath } from '@/app/paths';
import { useActions, useData, useServices } from '@/app/services';
import { Confirm } from '@/components/Confirm';
import { DateIcon } from '@/components/DateIcon';
import { FunnelArt, TagArt } from '@/components/Illustrations';
import { Menu } from '@/components/Menu';
import { useNow } from '@/lib/hooks';
import { useReorderDrag } from '@/lib/useReorderDrag';
import { useUi } from '@/store/ui';
import { reorderItems, tasksForView } from '@/features/tasks/organize';
import { assigneeMap, knownNames } from '../assignees/logic';
import { useAssignees } from '../assignees/store';
import { FilterDialog } from '../filters/FilterDialog';
import { useFilters } from '../filters/store';
import { TagDialog } from '../tags/TagDialog';
import { ListDialog } from './ListDialog';
/** TickTick's free tier allows nine lists; shown for parity but not enforced here. */
const LIST_LIMIT = 9;
/** One 38px, 8px-radius sidebar row: icon, name, a grey count, and a "..." menu on hover. */
function Row({ to, active, icon, label, count, menu, draggable, indicator }) {
    const [open, setOpen] = useState(false);
    const [at, setAt] = useState(null);
    const moreRef = useRef(null);
    const pointRef = useRef(null);
    return (_jsxs("div", { ...draggable, onContextMenu: menu ? (e) => { e.preventDefault(); setAt({ x: e.clientX, y: e.clientY }); } : undefined, className: `group relative flex h-[38px] items-center rounded-row ${active ? 'bg-selected' : 'hover:bg-hover'} ${indicator === 'before' ? 'before:absolute before:-top-px before:left-2 before:right-2 before:h-0.5 before:rounded before:bg-primary' : ''} ${indicator === 'after' ? 'after:absolute after:-bottom-px after:left-2 after:right-2 after:h-0.5 after:rounded after:bg-primary' : ''}`, children: [_jsxs(NavLink, { to: to, "aria-current": active ? 'page' : undefined, className: "flex h-full min-w-0 flex-1 items-center gap-2.5 rounded-row pl-3 pr-2", children: [_jsx("span", { className: "flex h-5 w-5 shrink-0 items-center justify-center", children: icon }), _jsx("span", { className: "min-w-0 flex-1 truncate", children: label }), count !== undefined && count > 0 && _jsx("span", { className: "text-s text-grey group-hover:hidden group-focus-within:hidden", children: count })] }), menu && (_jsxs(_Fragment, { children: [_jsx("button", { ref: moreRef, type: "button", "aria-label": `${label} options`, "aria-haspopup": "menu", onClick: () => setOpen(true), className: "mr-1 hidden h-6 w-6 items-center justify-center rounded text-grey hover:bg-black/5 group-focus-within:flex group-hover:flex", children: _jsx(MoreHorizontal, { size: 16 }) }), _jsx(Menu, { anchor: moreRef.current, open: open, onClose: () => setOpen(false), items: menu, label: `${label} options` }), _jsx("span", { ref: pointRef, "aria-hidden": true, style: { position: 'fixed', left: at?.x ?? 0, top: at?.y ?? 0, width: 0, height: 0 } }), _jsx(Menu, { anchor: at ? pointRef.current : null, open: !!at, onClose: () => setAt(null), items: menu, label: `${label} options` })] }))] }));
}
function SectionHeader({ label, collapsed, onToggle, badge, onAdd, addLabel }) {
    return (_jsxs("div", { className: "group mt-3 flex h-8 items-center px-3 text-s font-semibold text-grey", children: [_jsxs("button", { type: "button", onClick: onToggle, "aria-expanded": collapsed === undefined ? undefined : !collapsed, className: "flex items-center gap-1 rounded hover:text-text", children: [collapsed !== undefined && (collapsed ? _jsx(ChevronRight, { size: 14 }) : _jsx(ChevronDown, { size: 14 })), label] }), badge && _jsx("span", { className: "ml-2 rounded-full bg-black/5 px-2 py-px text-[11px] font-normal", children: badge }), onAdd && (_jsx("button", { type: "button", "aria-label": addLabel, onClick: onAdd, className: "ml-auto hidden h-6 w-6 items-center justify-center rounded hover:bg-black/5 group-focus-within:flex group-hover:flex", children: _jsx(Plus, { size: 16 }) }))] }));
}
export function Sidebar() {
    const navigate = useNavigate();
    const { pathname } = useLocation();
    const now = useNow(60_000);
    const actions = useActions();
    const { api } = useServices();
    const filters = useFilters((s) => s.items);
    const assigneeItems = useAssignees((s) => s.items);
    const assignees = useMemo(() => assigneeMap(assigneeItems), [assigneeItems]);
    const inboxId = useData((s) => s.inboxId);
    const listMap = useData((s) => s.lists);
    const taskMap = useData((s) => s.tasks);
    const tagMap = useData((s) => s.tags);
    const lists = useMemo(() => Object.values(listMap).sort((a, b) => a.sortOrder - b.sortOrder), [listMap]);
    const tags = useMemo(() => Object.values(tagMap).sort((a, b) => a.sortOrder - b.sortOrder), [tagMap]);
    const entities = useMemo(() => ({ tasks: Object.values(taskMap), lists, tags, inboxId, filters, assignees }), [taskMap, lists, tags, inboxId, filters, assignees]);
    useEffect(() => {
        void useFilters.getState().load(api, actions.notify);
        void useAssignees.getState().load(api, actions.notify);
    }, [api, actions.notify]);
    const isCollapsed = useUiSection();
    const count = (spec) => tasksForView(spec, entities, now).length;
    const userLists = lists.filter((l) => l.id !== inboxId && !l.archived);
    const archived = lists.filter((l) => l.archived);
    const active = (to) => pathname === to || pathname.startsWith(`${to}/`);
    const [listDialog, setListDialog] = useState(null);
    const [tagDialog, setTagDialog] = useState(null);
    const [filterDialog, setFilterDialog] = useState(null);
    const [deletingFilter, setDeletingFilter] = useState(null);
    const [deleting, setDeleting] = useState(null);
    const [deletingTag, setDeletingTag] = useState(null);
    const listDrag = useReorderDrag('application/x-tick-list', (moved, target, after) => {
        const r = reorderItems(userLists, moved, target, after);
        if (!r)
            return;
        if (r.kind === 'set')
            void actions.updateList(r.id, { sortOrder: r.sortOrder });
        else
            for (const [id, sortOrder] of Object.entries(r.orders))
                void actions.updateList(id, { sortOrder });
    });
    const tagDrag = useReorderDrag('application/x-tick-tag', (moved, target, after) => {
        const r = reorderItems(tags.map((t) => ({ id: t.name, sortOrder: t.sortOrder })), moved, target, after);
        if (!r)
            return;
        if (r.kind === 'set')
            void actions.updateTag(moved, { sortOrder: r.sortOrder });
        else
            for (const [name, sortOrder] of Object.entries(r.orders))
                void actions.updateTag(name, { sortOrder });
    });
    const listMenu = (l) => [
        { id: 'edit', label: 'Edit', icon: _jsx(Pencil, { size: 16 }), onSelect: () => setListDialog({ list: l }) },
        l.archived
            ? { id: 'unarchive', label: 'Unarchive', icon: _jsx(Undo2, { size: 16 }), onSelect: () => void actions.updateList(l.id, { archived: false }) }
            : { id: 'archive', label: 'Archive', icon: _jsx(Archive, { size: 16 }), onSelect: () => void actions.updateList(l.id, { archived: true }) },
        'separator',
        { id: 'delete', label: 'Delete', icon: _jsx(Trash2, { size: 16 }), danger: true, onSelect: () => setDeleting(l) },
    ];
    const filterMenu = (f) => [
        { id: 'edit', label: 'Edit', icon: _jsx(Pencil, { size: 16 }), onSelect: () => setFilterDialog({ filter: f }) },
        'separator',
        { id: 'delete', label: 'Delete', icon: _jsx(Trash2, { size: 16 }), danger: true, onSelect: () => setDeletingFilter(f) },
    ];
    const tagMenu = (t) => [
        { id: 'edit', label: 'Edit', icon: _jsx(Pencil, { size: 16 }), onSelect: () => setTagDialog({ tag: t }) },
        'separator',
        { id: 'delete', label: 'Delete', icon: _jsx(Trash2, { size: 16 }), danger: true, onSelect: () => setDeletingTag(t) },
    ];
    const listIcon = (l) => _jsx(AlignJustify, { size: 18, style: l.color ? { color: l.color } : undefined });
    const dot = (t) => _jsx("span", { className: "h-2.5 w-2.5 rounded-full", style: { backgroundColor: t.color ?? 'rgb(var(--grey))' } });
    const marker = (d, id) => (d.indicator?.id === id ? (d.indicator.after ? 'after' : 'before') : null);
    return (_jsxs("aside", { "aria-label": "Lists", className: "flex h-full w-[240px] shrink-0 flex-col bg-side", children: [_jsxs("div", { className: "scroll-thin min-h-0 flex-1 overflow-y-auto px-2 pb-2 pt-3", children: [_jsx(Row, { to: "/q/all/tasks", active: active('/q/all/tasks'), icon: _jsx(Layers, { size: 18 }), label: "All", count: count({ kind: 'all' }) }), _jsx(Row, { to: "/q/today/tasks", active: active('/q/today/tasks'), icon: _jsx(DateIcon, { label: String(new Date(now).getDate()) }), label: "Today", count: count({ kind: 'today' }) }), _jsx(Row, { to: "/q/week/tasks", active: active('/q/week/tasks'), icon: _jsx(DateIcon, { label: new Date(now).toLocaleDateString('en-US', { weekday: 'short' }).slice(0, 2) }), label: "Next 7 Days", count: count({ kind: 'week' }) }), _jsx(Row, { to: "/p/inbox/tasks", active: active('/p/inbox/tasks'), icon: _jsx(Inbox, { size: 18 }), label: "Inbox", count: count({ kind: 'inbox' }) }), _jsx(Row, { to: PATHS.summary, active: active(PATHS.summary), icon: _jsx(FileText, { size: 18 }), label: "Summary" }), _jsx(SectionHeader, { label: "Lists", badge: `Used: ${userLists.length + archived.length}/${LIST_LIMIT}`, onAdd: () => setListDialog({ list: null }), addLabel: "Add list" }), userLists.map((l) => (_jsx(Row, { to: viewPath({ kind: 'list', id: l.id }), active: active(viewPath({ kind: 'list', id: l.id })), icon: listIcon(l), label: l.name, count: count({ kind: 'list', id: l.id }), menu: listMenu(l), draggable: listDrag.bind(l.id), indicator: marker(listDrag, l.id) }, l.id))), archived.length > 0 && (_jsxs(_Fragment, { children: [_jsx(SectionHeader, { label: "Archived Lists", collapsed: isCollapsed('archived'), onToggle: () => isCollapsed.toggle('archived') }), isCollapsed('archived') ? null : archived.map((l) => (_jsx(Row, { to: viewPath({ kind: 'list', id: l.id }), active: active(viewPath({ kind: 'list', id: l.id })), icon: _jsx(Archive, { size: 18 }), label: l.name, menu: listMenu(l) }, l.id)))] })), _jsx(SectionHeader, { label: "Filters", onAdd: () => setFilterDialog({ filter: null }), addLabel: "Add filter" }), filters.length === 0 ? (_jsxs("div", { className: "mx-1 flex items-center gap-3 rounded-row bg-black/[.03] px-3 py-3 text-s text-grey", children: [_jsx(FunnelArt, {}), _jsx("p", { children: "Display tasks filtered by list, date, priority, tag, and more" })] })) : (filters.map((f) => (_jsx(Row, { to: viewPath({ kind: 'filter', id: f.id }), active: active(viewPath({ kind: 'filter', id: f.id })), icon: _jsx(FilterIcon, { size: 18 }), label: f.name, count: count({ kind: 'filter', id: f.id }), menu: filterMenu(f) }, f.id)))), _jsx(SectionHeader, { label: "Tags", onAdd: () => setTagDialog({ tag: null }), addLabel: "Add tag" }), tags.length === 0 ? (_jsxs("div", { className: "mx-1 flex items-center gap-3 rounded-row bg-black/[.03] px-3 py-3 text-s text-grey", children: [_jsx(TagArt, {}), _jsx("p", { children: "Categorize your tasks with tags. Quickly select a tag by typing \"#\" when adding a task" })] })) : (tags.map((t) => (_jsx(Row, { to: viewPath({ kind: 'tag', name: t.name }), active: active(viewPath({ kind: 'tag', name: t.name })), icon: dot(t), label: t.label, count: count({ kind: 'tag', name: t.name }), menu: tagMenu(t), draggable: tagDrag.bind(t.name), indicator: marker(tagDrag, t.name) }, t.name))))] }), _jsxs("div", { className: "border-t border-line px-2 py-2", children: [_jsx(Row, { to: PATHS.completed, active: active(PATHS.completed), icon: _jsx(CheckCircle2, { size: 18 }), label: "Completed" }), _jsx(Row, { to: PATHS.trash, active: active(PATHS.trash), icon: _jsx(Trash2, { size: 18 }), label: "Trash" }), _jsxs("button", { type: "button", disabled: true, title: "Not applicable in Tick Local", className: "mt-1 flex h-9 w-full cursor-not-allowed items-center gap-2 rounded-row bg-black/5 px-3 text-s text-grey/80", children: [_jsx(Crown, { size: 16 }), _jsx("span", { className: "flex-1 text-left", children: "Upgrade to Premium" }), _jsx(ChevronRight, { size: 14 })] })] }), _jsx(ListDialog, { open: !!listDialog, list: listDialog?.list ?? null, onClose: () => setListDialog(null), onSubmit: (input) => {
                    const editing = listDialog?.list;
                    setListDialog(null);
                    if (editing)
                        void actions.updateList(editing.id, input);
                    else
                        void actions.createList(input).then((l) => navigate(viewPath({ kind: 'list', id: l.id }))).catch((e) => actions.notify('error', e.message));
                } }), _jsx(FilterDialog, { open: !!filterDialog, filter: filterDialog?.filter ?? null, lists: lists, tags: tags, assigneeNames: knownNames(assigneeItems), onClose: () => setFilterDialog(null), onSubmit: (input) => {
                    const editing = filterDialog?.filter;
                    setFilterDialog(null);
                    if (editing)
                        useFilters.getState().put(editing.id, input);
                    else
                        navigate(viewPath({ kind: 'filter', id: useFilters.getState().add(input) }));
                } }), _jsx(Confirm, { open: !!deletingFilter, title: "Delete filter?", message: `"${deletingFilter?.name}" will be deleted. Its tasks are not affected.`, confirmLabel: "Delete", danger: true, onCancel: () => setDeletingFilter(null), onConfirm: () => {
                    const filter = deletingFilter;
                    setDeletingFilter(null);
                    if (!filter)
                        return;
                    if (pathname.startsWith(`/f/${filter.id}/`))
                        navigate('/q/all/tasks');
                    useFilters.getState().remove(filter.id);
                } }), _jsx(TagDialog, { open: !!tagDialog, tag: tagDialog?.tag ?? null, onClose: () => setTagDialog(null), onSubmit: ({ label, color }) => {
                    const editing = tagDialog?.tag;
                    setTagDialog(null);
                    const fail = (e) => actions.notify('error', e.message);
                    if (!editing)
                        return void actions.createTag(label, color).catch(fail);
                    void (async () => {
                        if (label !== editing.label)
                            await actions.renameTag(editing.name, label);
                        if (color !== editing.color)
                            await actions.updateTag(label.toLowerCase(), { color });
                    })().catch(fail);
                } }), _jsx(Confirm, { open: !!deleting, title: "Delete list?", message: `"${deleting?.name}" will be deleted. Its tasks move to the Trash.`, confirmLabel: "Delete", danger: true, onCancel: () => setDeleting(null), onConfirm: () => {
                    const list = deleting;
                    setDeleting(null);
                    if (!list)
                        return;
                    if (pathname.startsWith(`/p/${list.id}/`))
                        navigate('/q/all/tasks');
                    void actions.deleteList(list.id);
                } }), _jsx(Confirm, { open: !!deletingTag, title: "Delete tag?", message: `"${deletingTag?.label}" will be removed from every task.`, confirmLabel: "Delete", danger: true, onCancel: () => setDeletingTag(null), onConfirm: () => {
                    const tag = deletingTag;
                    setDeletingTag(null);
                    if (!tag)
                        return;
                    if (pathname.startsWith(`/t/${encodeURIComponent(tag.name)}/`))
                        navigate('/q/all/tasks');
                    void actions.deleteTag(tag.name);
                } })] }));
}
/** Collapse state for sidebar sections, from the UI store. */
function useUiSection() {
    const sections = useUi((s) => s.sections);
    const toggleSection = useUi((s) => s.toggleSection);
    const isCollapsed = (id) => !!sections[id];
    isCollapsed.toggle = toggleSection;
    return isCollapsed;
}

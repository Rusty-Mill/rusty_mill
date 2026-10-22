import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { Search, X } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { PATHS, taskPath, viewPath } from '@/app/paths';
import { useData } from '@/app/services';
import { Dialog } from '@/components/Dialog';
import { useUi } from '@/store/ui';
import { highlight, searchLists, searchTasks } from './search';
function Marked({ text, query }) {
    return (_jsx(_Fragment, { children: highlight(text, query).map((s, i) => s.hit ? (_jsx("mark", { className: "rounded-xs bg-mark text-text", children: s.text }, i)) : (s.text)) }));
}
/** Ctrl/⌘+K: find a task or a list. Arrow keys move, Enter opens. */
export function SearchModal() {
    const open = useUi((s) => s.searchOpen);
    const setOpen = useUi((s) => s.setSearchOpen);
    return (_jsxs(Dialog, { open: open, onClose: () => setOpen(false), width: 560, labelledBy: "search-title", bare: true, children: [_jsx("h2", { id: "search-title", className: "sr-only", children: "Search" }), open && _jsx(SearchBody, { onDone: () => setOpen(false) })] }));
}
function SearchBody({ onDone }) {
    const navigate = useNavigate();
    const inboxId = useData((s) => s.inboxId);
    const taskMap = useData((s) => s.tasks);
    const listMap = useData((s) => s.lists);
    const [query, setQuery] = useState('');
    const [mode, setMode] = useState('task');
    const [active, setActive] = useState(0);
    const listId = useId();
    const box = useRef(null);
    const tasks = useMemo(() => Object.values(taskMap), [taskMap]);
    const lists = useMemo(() => Object.values(listMap), [listMap]);
    const taskHits = useMemo(() => (mode === 'task' ? searchTasks(query, tasks) : []), [mode, query, tasks]);
    const listHits = useMemo(() => (mode === 'list' ? searchLists(query, lists) : []), [mode, query, lists]);
    const count = mode === 'task' ? taskHits.length : listHits.length;
    const activeIndex = Math.min(active, Math.max(count - 1, 0));
    useEffect(() => setActive(0), [query, mode]);
    useEffect(() => {
        box.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' });
    }, [activeIndex]);
    const openTask = (i) => {
        const hit = taskHits[i];
        if (!hit)
            return;
        const t = hit.task;
        onDone();
        if (t.status !== 'open')
            navigate(`${PATHS.completed}/${t.id}`);
        else
            navigate(taskPath(t.listId === inboxId ? { kind: 'inbox' } : { kind: 'list', id: t.listId }, t.id));
    };
    const openList = (i) => {
        const l = listHits[i];
        if (!l)
            return;
        onDone();
        navigate(viewPath(l.id === inboxId ? { kind: 'inbox' } : { kind: 'list', id: l.id }));
    };
    const choose = (i) => (mode === 'task' ? openTask(i) : openList(i));
    const onKeyDown = (e) => {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            if (count > 0)
                setActive((a) => (Math.min(a, count - 1) + (e.key === 'ArrowDown' ? 1 : -1) + count) % count);
        }
        else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault();
            choose(activeIndex);
        }
    };
    return (_jsxs("div", { className: "flex max-h-[70vh] flex-col", onKeyDown: onKeyDown, children: [_jsxs("div", { className: "flex h-14 shrink-0 items-center gap-3 border-b border-line px-4", children: [_jsx(Search, { size: 18, className: "text-grey", "aria-hidden": true }), _jsx("input", { "data-autofocus": true, role: "combobox", "aria-expanded": "true", "aria-controls": listId, "aria-activedescendant": count > 0 ? `${listId}-${activeIndex}` : undefined, "aria-label": "Search", autoComplete: "off", spellCheck: false, value: query, onChange: (e) => setQuery(e.target.value), placeholder: mode === 'task' ? 'Search tasks' : 'Search lists', className: "h-full min-w-0 flex-1 bg-transparent text-[16px] outline-hidden placeholder:text-grey" }), query && (_jsx("button", { type: "button", "aria-label": "Clear search", onClick: () => setQuery(''), className: "flex h-6 w-6 items-center justify-center rounded-full text-grey hover:bg-hover", children: _jsx(X, { size: 14 }) })), _jsx("button", { type: "button", "aria-label": "Close search", onClick: onDone, className: "flex h-7 w-7 items-center justify-center rounded-row text-grey hover:bg-hover", children: _jsx(X, { size: 18 }) })] }), _jsx("div", { role: "radiogroup", "aria-label": "Search in", className: "flex shrink-0 gap-2 px-4 py-2", children: ['task', 'list'].map((m) => (_jsx("button", { type: "button", role: "radio", "aria-checked": mode === m, onClick: () => setMode(m), className: `h-7 rounded-full px-3 text-base ${mode === m ? 'bg-primary/10 font-semibold text-primary' : 'bg-black/[.05] text-grey hover:text-text'}`, children: m === 'task' ? 'Task' : 'List' }, m))) }), _jsxs("ul", { ref: box, id: listId, role: "listbox", "aria-label": "Results", className: "scroll-thin min-h-[120px] flex-1 overflow-y-auto px-2 pb-3", children: [count > 0 && _jsx("li", { role: "presentation", className: "px-3 pb-1 pt-2 text-s font-semibold", children: mode === 'task' ? 'Tasks' : 'Lists' }), count === 0 && _jsx("li", { role: "presentation", className: "px-3 py-8 text-center text-grey", children: query.trim() ? 'No results' : mode === 'task' ? 'Nothing here yet' : 'No lists' }), mode === 'task' &&
                        taskHits.map(({ task, snippet }, i) => (_jsxs("li", { id: `${listId}-${i}`, role: "option", "aria-selected": i === activeIndex, onMouseMove: () => setActive(i), onClick: () => openTask(i), className: `flex cursor-pointer items-center gap-3 rounded-row px-3 py-2 ${i === activeIndex ? 'bg-selected' : ''}`, children: [_jsxs("div", { className: "min-w-0 flex-1", children: [_jsx("p", { className: `truncate ${task.status !== 'open' ? 'text-grey line-through' : ''}`, children: _jsx(Marked, { text: task.title, query: query }) }), snippet && (_jsx("p", { className: "truncate text-s text-grey", children: _jsx(Marked, { text: snippet, query: query }) }))] }), _jsx("span", { className: "shrink-0 text-s text-grey", children: listMap[task.listId]?.name })] }, task.id))), mode === 'list' &&
                        listHits.map((l, i) => (_jsxs("li", { id: `${listId}-${i}`, role: "option", "aria-selected": i === activeIndex, onMouseMove: () => setActive(i), onClick: () => openList(i), className: `flex cursor-pointer items-center gap-3 rounded-row px-3 py-2 ${i === activeIndex ? 'bg-selected' : ''}`, children: [_jsx("span", { className: "h-2.5 w-2.5 shrink-0 rounded-full", style: { backgroundColor: l.color ?? 'rgb(var(--grey))' } }), _jsx("span", { className: "min-w-0 flex-1 truncate", children: _jsx(Marked, { text: l.name, query: query }) })] }, l.id)))] }), _jsxs("p", { className: "shrink-0 pb-3 text-center text-s text-grey", children: ["Not found yet? ", _jsx("span", { className: "text-primary/60", title: "Cloud search is not available in Tick Local", children: "Try Cloud Search." })] })] }));
}

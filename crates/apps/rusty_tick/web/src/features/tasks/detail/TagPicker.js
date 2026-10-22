import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Check, Plus, Tag as TagIcon } from 'lucide-react';
import { useMemo, useState } from 'react';
import { Popover } from '@/components/Popover';
/** Search-as-you-type tag picker. Enter takes the first match, or creates the typed tag. */
export function TagPicker({ anchor, open, onClose, tags, selected, onToggle, onCreate }) {
    return (_jsx(Popover, { anchor: anchor, open: open, onClose: onClose, ariaLabel: "Tags", role: "dialog", className: "w-64 p-2", children: open && _jsx(Body, { tags: tags, selected: selected, onToggle: onToggle, onCreate: onCreate }) }));
}
function Body({ tags, selected, onToggle, onCreate }) {
    const [query, setQuery] = useState('');
    const q = query.trim().toLowerCase();
    const matches = useMemo(() => tags.filter((t) => t.label.toLowerCase().includes(q)), [tags, q]);
    const exact = tags.some((t) => t.name === q);
    const canCreate = q !== '' && !exact;
    const onEnter = () => {
        const first = matches[0];
        if (exact || (first && !canCreate))
            onToggle((exact ? tags.find((t) => t.name === q) : first).name);
        else if (canCreate)
            onCreate(query.trim());
        setQuery('');
    };
    return (_jsxs("div", { children: [_jsx("input", { "data-autofocus": true, value: query, maxLength: 64, onChange: (e) => setQuery(e.target.value), onKeyDown: (e) => {
                    if (e.key === 'Enter') {
                        e.preventDefault();
                        onEnter();
                    }
                    else if (e.key === 'ArrowDown') {
                        e.preventDefault();
                        e.currentTarget.parentElement?.querySelector('[role="option"]')?.focus();
                    }
                }, placeholder: "Search or create a tag", "aria-label": "Search or create a tag", className: "mb-1.5 h-8 w-full rounded-row border border-line bg-surface px-2 text-base outline-hidden focus:border-primary" }), _jsx("ul", { role: "listbox", "aria-label": "Tags", "aria-multiselectable": true, className: "scroll-thin max-h-56 overflow-y-auto", children: matches.map((t) => {
                    const on = selected.includes(t.name);
                    return (_jsx("li", { role: "none", children: _jsxs("button", { type: "button", role: "option", "aria-selected": on, onClick: () => onToggle(t.name), onKeyDown: (e) => {
                                const li = e.currentTarget.closest('li');
                                if (e.key === 'ArrowDown')
                                    li?.nextElementSibling?.querySelector('button')?.focus();
                                if (e.key === 'ArrowUp')
                                    (li?.previousElementSibling?.querySelector('button') ?? e.currentTarget.closest('div')?.querySelector('input'))?.focus();
                            }, className: "flex h-8 w-full items-center gap-2 rounded-row px-2 text-left hover:bg-hover", children: [_jsx("span", { className: "h-2.5 w-2.5 shrink-0 rounded-full", style: { backgroundColor: t.color ?? 'rgb(var(--grey))' } }), _jsx("span", { className: "flex-1 truncate", children: t.label }), on && _jsx(Check, { size: 16, className: "text-primary", "aria-hidden": true })] }) }, t.name));
                }) }), canCreate && (_jsxs("button", { type: "button", onClick: onEnter, className: "mt-1 flex h-8 w-full items-center gap-2 rounded-row px-2 text-left text-primary hover:bg-hover", children: [_jsx(Plus, { size: 16 }), _jsxs("span", { className: "truncate", children: ["Create tag \u201C", query.trim(), "\u201D"] })] })), matches.length === 0 && !canCreate && (_jsxs("p", { className: "flex items-center gap-2 px-2 py-2 text-s text-grey", children: [_jsx(TagIcon, { size: 14 }), " No tags yet. Type a name to make one."] }))] }));
}

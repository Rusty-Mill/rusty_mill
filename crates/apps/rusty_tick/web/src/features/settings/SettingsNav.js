import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Bell, Bot, Clock, Crown, Info, Keyboard, LayoutGrid, ListFilter, MoreHorizontal, Palette, Plug, User, Users } from 'lucide-react';
export const TAB_META = [
    { id: 'account', label: 'Account', icon: User },
    { id: 'premium', label: 'Premium', icon: Crown },
    { id: 'features', label: 'Features', icon: LayoutGrid },
    { id: 'smart-list', label: 'Smart List', icon: ListFilter },
    { id: 'notifications', label: 'Notifications', icon: Bell },
    { id: 'date-time', label: 'Date & Time', icon: Clock },
    { id: 'appearance', label: 'Appearance', icon: Palette },
    { id: 'ai', label: 'AI Features', icon: Bot },
    { id: 'more', label: 'More', icon: MoreHorizontal },
    { id: 'integrations', label: 'Integrations & Import', icon: Plug },
    { id: 'collaborate', label: 'Collaborate', icon: Users },
    { id: 'shortcuts', label: 'Shortcuts', icon: Keyboard },
    { id: 'about', label: 'About', icon: Info },
];
export const tabId = (id) => `settings-tab-${id}`;
export const panelId = (id) => `settings-panel-${id}`;
/** A vertical tablist: one tab stop, arrows/Home/End move and select (the panel is cheap, so selection follows focus). */
export function SettingsNav({ selected, onSelect }) {
    const move = (delta) => {
        const at = TAB_META.findIndex((t) => t.id === selected);
        const next = delta === 'first' ? 0 : delta === 'last' ? TAB_META.length - 1 : (at + delta + TAB_META.length) % TAB_META.length;
        const tab = TAB_META[next];
        if (!tab)
            return;
        onSelect(tab.id);
        requestAnimationFrame(() => document.getElementById(tabId(tab.id))?.focus());
    };
    return (_jsx("div", { role: "tablist", "aria-label": "Settings", "aria-orientation": "vertical", className: "flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-3", onKeyDown: (e) => {
            const key = { ArrowDown: 1, ArrowUp: -1, Home: 'first', End: 'last' };
            const delta = key[e.key];
            if (delta === undefined)
                return;
            e.preventDefault();
            move(delta);
        }, children: TAB_META.map((t) => {
            const on = t.id === selected;
            return (_jsxs("button", { id: tabId(t.id), type: "button", role: "tab", "aria-selected": on, "aria-controls": panelId(t.id), tabIndex: on ? 0 : -1, "data-autofocus": on || undefined, onClick: () => onSelect(t.id), className: `flex h-9 shrink-0 items-center gap-2.5 rounded-row px-2.5 text-left text-base outline-hidden focus-visible:ring-2 focus-visible:ring-primary ${on ? 'bg-selected font-semibold' : 'hover:bg-hover'}`, children: [_jsx(t.icon, { size: 16, strokeWidth: 1.5, className: "shrink-0 text-grey", "aria-hidden": true }), _jsx("span", { className: "truncate", children: t.label })] }, t.id));
        }) }));
}

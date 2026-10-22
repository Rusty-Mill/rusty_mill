import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useUi } from '@/store/ui';
import { usePrefs } from '../prefs';
import { PaneTitle, Row, Switch } from './controls';
const THEMES = [
    { value: 'light', label: 'Light' },
    { value: 'dark', label: 'Dark' },
    { value: 'system', label: 'System' },
];
/** A miniature window in a theme's own colours (fixed values: it must show that theme, whichever one is active). */
function Preview({ theme }) {
    const c = theme === 'dark' ? { bg: '#1a1a1c', side: '#1e1e20', line: '#303032', bar: '#8c8c8c' } : { bg: '#ffffff', side: '#f7f7f7', line: '#f0f1f2', bar: '#c8c8c8' };
    return (_jsxs("span", { "aria-hidden": true, className: "flex h-[54px] w-full overflow-hidden rounded-md border", style: { background: c.bg, borderColor: c.line }, children: [_jsx("span", { className: "w-[28%]", style: { background: c.side } }), _jsxs("span", { className: "flex flex-1 flex-col gap-1.5 p-2", children: [_jsx("span", { className: "h-1.5 w-3/4 rounded-full", style: { background: c.bar } }), _jsx("span", { className: "h-1.5 w-1/2 rounded-full", style: { background: c.bar } }), _jsx("span", { className: "h-1.5 w-2/3 rounded-full", style: { background: '#4772FA' } })] })] }));
}
export function AppearanceTab() {
    const theme = usePrefs((s) => s.prefs.theme);
    const update = usePrefs((s) => s.update);
    const collapsed = useUi((s) => s.sidebarCollapsed);
    const toggleSidebar = useUi((s) => s.toggleSidebar);
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "Appearance" }), _jsx("div", { role: "radiogroup", "aria-label": "Theme", className: "grid grid-cols-3 gap-3 border-b border-line pb-5", children: THEMES.map((t) => (_jsxs("label", { className: "cursor-pointer", children: [_jsx("input", { type: "radio", name: "theme", checked: theme === t.value, onChange: () => update({ theme: t.value }), className: "peer sr-only" }), _jsxs("span", { className: "flex flex-col gap-2 rounded-row border border-line p-2 peer-checked:border-primary peer-checked:ring-1 peer-checked:ring-primary peer-focus-visible:ring-2 peer-focus-visible:ring-primary", children: [t.value === 'system' ? (_jsxs("span", { "aria-hidden": true, className: "flex gap-1", children: [_jsx(Preview, { theme: "light" }), _jsx(Preview, { theme: "dark" })] })) : (_jsx(Preview, { theme: t.value })), _jsx("span", { className: "text-center text-base", children: t.label })] })] }, t.value))) }), _jsx(Row, { label: "Show sidebar", hint: "The list of smart lists, lists and tags beside your tasks.", htmlFor: "show-sidebar", children: _jsx(Switch, { id: "show-sidebar", label: "Show sidebar", checked: !collapsed, onChange: () => toggleSidebar() }) })] }));
}

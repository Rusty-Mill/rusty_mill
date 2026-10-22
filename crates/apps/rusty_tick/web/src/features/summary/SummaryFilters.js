import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { RANGE_KEYS, RANGE_LABELS } from './range';
import { TEMPLATES, TEMPLATE_LABELS } from './options';
const PRIORITIES = [
    { value: 5, label: 'High' },
    { value: 3, label: 'Medium' },
    { value: 1, label: 'Low' },
    { value: 0, label: 'None' },
];
const STATUSES = [
    { value: 'all', label: 'All Status' },
    { value: 'completed', label: 'Completed' },
    { value: 'open', label: 'Not completed' },
];
/** Full names do not fit three abreast; the accessible name stays the full one. */
const TEMPLATE_SHORT = { daily: 'Daily', weekly: 'Weekly', simple: 'Simple list' };
const FIELDS = [
    { key: 'showList', label: 'Show list name' },
    { key: 'showDue', label: 'Show due date' },
    { key: 'showPriority', label: 'Show priority' },
    { key: 'showTags', label: 'Show tags' },
    { key: 'showCompletedTime', label: 'Show completed time' },
];
const toggle = (xs, x) => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x]);
function Section({ title, children }) {
    return (_jsxs("section", { className: "flex flex-col gap-2 px-5 py-3", children: [_jsx("h3", { className: "text-base font-semibold", children: title }), _jsx("div", { className: "flex flex-col rounded-[10px] bg-side px-4 py-1", children: children })] }));
}
function Check({ label, checked, onChange }) {
    return (_jsxs("label", { className: "flex min-h-7 cursor-pointer items-center gap-2 text-base", children: [_jsx("input", { type: "checkbox", checked: checked, onChange: (e) => onChange(e.target.checked), className: "h-4 w-4 accent-primary" }), _jsx("span", { className: "min-w-0 flex-1 truncate", children: label })] }));
}
const rowClass = 'flex min-h-10 items-center justify-between gap-3 text-base';
const valueClass = 'h-8 min-w-0 max-w-[190px] cursor-pointer bg-transparent text-right text-grey outline-hidden';
/** A label on the left and a select on the right, as the real app lays out its filters. */
function SelectRow({ label, value, onChange, children }) {
    return (_jsxs("label", { className: rowClass, children: [label, _jsx("select", { value: value, onChange: (e) => onChange(e.target.value), className: valueClass, children: children })] }));
}
/** A row whose value ("All Lists", "3 selected") opens its choices underneath. */
function OpenRow({ label, value, children }) {
    return (_jsxs("details", { className: "group", children: [_jsxs("summary", { className: `${rowClass} cursor-pointer list-none [&::-webkit-details-marker]:hidden`, children: [label, _jsxs("span", { className: "truncate text-grey", children: [value, " \u25BE"] })] }), _jsx("div", { className: "mb-2 flex flex-col", children: children })] }));
}
const field = 'h-8 w-full rounded-row border border-line bg-surface px-2 text-base outline-hidden focus:border-primary';
/** Template, Filter (range, lists, status, more) and Display Options: the summary's right-hand panel. */
export function SummaryFilters({ options: o, onChange, lists, tags }) {
    const shownLists = lists.filter((l) => !l.archived);
    return (_jsxs("div", { children: [_jsx(Section, { title: "Template", children: _jsx("div", { role: "radiogroup", "aria-label": "Template", className: "flex gap-1 rounded-row bg-selected p-0.5", children: TEMPLATES.map((t) => (_jsx("button", { type: "button", role: "radio", "aria-label": TEMPLATE_LABELS[t], "aria-checked": o.template === t, onClick: () => onChange({ template: t }), className: `h-7 flex-1 rounded-[6px] px-1 text-s ${o.template === t ? 'bg-surface font-semibold text-text shadow-xs' : 'text-grey hover:text-text'}`, children: TEMPLATE_SHORT[t] }, t))) }) }), _jsxs(Section, { title: "Filter", children: [_jsx(SelectRow, { label: "Date", value: o.range, onChange: (v) => onChange({ range: v }), children: RANGE_KEYS.map((k) => (_jsx("option", { value: k, children: RANGE_LABELS[k] }, k))) }), o.range === 'custom' && (_jsxs("div", { className: "flex items-center gap-2 pb-2", children: [_jsx("input", { type: "date", "aria-label": "From date", value: o.custom.from, onChange: (e) => onChange({ custom: { ...o.custom, from: e.target.value } }), className: field }), _jsx("span", { className: "text-grey", "aria-hidden": true, children: "\u2013" }), _jsx("input", { type: "date", "aria-label": "To date", value: o.custom.to, onChange: (e) => onChange({ custom: { ...o.custom, to: e.target.value } }), className: field })] })), _jsx(OpenRow, { label: "Lists", value: o.listIds.length === 0 ? 'All Lists' : `${o.listIds.length} selected`, children: _jsxs("div", { className: "max-h-[132px] overflow-y-auto", children: [_jsx(Check, { label: "All lists", checked: o.listIds.length === 0, onChange: () => onChange({ listIds: [] }) }), shownLists.map((l) => (_jsx(Check, { label: l.name, checked: o.listIds.includes(l.id), onChange: () => onChange({ listIds: toggle(o.listIds, l.id) }) }, l.id)))] }) }), _jsx(SelectRow, { label: "Status", value: o.status, onChange: (v) => onChange({ status: v }), children: STATUSES.map((s) => (_jsx("option", { value: s.value, children: s.label }, s.value))) }), _jsxs(OpenRow, { label: "More", value: o.priorities.length + o.tags.length === 0 ? 'None' : `${o.priorities.length + o.tags.length} selected`, children: [_jsxs("fieldset", { className: "flex flex-col", children: [_jsx("legend", { className: "mb-1 text-s text-grey", children: "Priority" }), PRIORITIES.map((p) => (_jsx(Check, { label: p.label, checked: o.priorities.includes(p.value), onChange: () => onChange({ priorities: toggle(o.priorities, p.value) }) }, p.value)))] }), _jsxs("fieldset", { className: "mt-2 flex flex-col", children: [_jsx("legend", { className: "mb-1 text-s text-grey", children: "Tags" }), tags.length === 0 && _jsx("p", { className: "text-s text-grey", children: "No tags yet." }), _jsx("div", { className: "max-h-[110px] overflow-y-auto", children: tags.map((g) => (_jsx(Check, { label: `#${g.label}`, checked: o.tags.includes(g.name), onChange: () => onChange({ tags: toggle(o.tags, g.name) }) }, g.name))) })] })] })] }), _jsxs(Section, { title: "Display Options", children: [_jsxs(SelectRow, { label: "Grouping", value: o.groupBy, onChange: (v) => onChange({ groupBy: v }), children: [_jsx("option", { value: "none", children: "No Grouping" }), _jsx("option", { value: "list", children: "By List" })] }), _jsx(OpenRow, { label: "Fields", value: `${FIELDS.filter((f) => o[f.key]).length} selected`, children: FIELDS.map((f) => (_jsx(Check, { label: f.label, checked: o[f.key], onChange: (v) => onChange({ [f.key]: v }) }, f.key))) })] })] }));
}

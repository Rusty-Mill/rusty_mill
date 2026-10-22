import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { Dialog } from '@/components/Dialog';
import { UNASSIGNED } from '../assignees/logic';
import { DATE_BUCKETS, PRIORITIES, emptyRule } from './logic';
export function FilterDialog({ open, onClose, filter, lists, tags, assigneeNames, onSubmit }) {
    return (_jsx(Dialog, { open: open, onClose: onClose, title: filter ? 'Edit Filter' : 'Add Filter', width: 460, children: open && _jsx(FilterForm, { filter: filter, lists: lists, tags: tags, assigneeNames: assigneeNames, onCancel: onClose, onSubmit: onSubmit }, filter?.id ?? 'new') }));
}
const toggle = (xs, x) => (xs.includes(x) ? xs.filter((y) => y !== x) : [...xs, x]);
function FilterForm({ filter, lists, tags, assigneeNames, onCancel, onSubmit }) {
    const [name, setName] = useState(filter?.name ?? '');
    const [rule, setRule] = useState(filter?.rule ?? emptyRule());
    const valid = name.trim().length > 0;
    const submit = (e) => {
        e.preventDefault();
        if (valid)
            onSubmit({ name: name.trim(), rule });
    };
    return (_jsxs("form", { onSubmit: submit, className: "flex flex-col gap-4 px-6 pb-6 pt-2", children: [_jsxs("label", { className: "flex flex-col gap-1.5", children: [_jsx("span", { className: "text-s text-grey", children: "Name" }), _jsx("input", { "data-autofocus": true, value: name, maxLength: 200, onChange: (e) => setName(e.target.value), placeholder: "Filter name", className: "h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), _jsx(Chips, { legend: "Lists", options: lists.filter((l) => !l.archived).map((l) => [l.id, l.name]), selected: rule.lists, onToggle: (v) => setRule({ ...rule, lists: toggle(rule.lists, v) }) }), _jsx(Chips, { legend: "Tags", options: tags.map((t) => [t.name, t.label]), selected: rule.tags, onToggle: (v) => setRule({ ...rule, tags: toggle(rule.tags, v) }) }), _jsx(Chips, { legend: "Priority", options: PRIORITIES.map(([p, l]) => [String(p), l]), selected: rule.priorities.map(String), onToggle: (v) => setRule({ ...rule, priorities: toggle(rule.priorities, Number(v)) }) }), _jsx(Chips, { legend: "Assignee", options: assigneeNames.length ? [...assigneeNames, UNASSIGNED].map((n) => [n, n === UNASSIGNED ? 'Unassigned' : n]) : [], selected: rule.assignees, onToggle: (v) => setRule({ ...rule, assignees: toggle(rule.assignees, v) }) }), _jsx(Chips, { legend: "Date", options: DATE_BUCKETS, selected: rule.dates, onToggle: (v) => setRule({ ...rule, dates: toggle(rule.dates, v) }) }), _jsx("p", { className: "text-s text-grey", children: "Each group narrows the result; leave a group empty to ignore it." }), _jsxs("div", { className: "flex justify-end gap-2 pt-1", children: [_jsx("button", { type: "button", onClick: onCancel, className: "h-8 rounded-row px-4 hover:bg-hover", children: "Cancel" }), _jsx("button", { type: "submit", disabled: !valid, className: "h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40", children: filter ? 'Save' : 'Add' })] })] }));
}
function Chips({ legend, options, selected, onToggle }) {
    if (options.length === 0)
        return null;
    return (_jsxs("fieldset", { className: "flex flex-col gap-1.5", children: [_jsx("legend", { className: "mb-1.5 text-s text-grey", children: legend }), _jsx("div", { className: "flex flex-wrap gap-2", children: options.map(([value, label]) => (_jsx("button", { type: "button", "aria-pressed": selected.includes(value), onClick: () => onToggle(value), className: `h-7 rounded-full border px-3 text-s ${selected.includes(value) ? 'border-primary bg-primary/10 text-primary' : 'border-line text-grey hover:bg-hover'}`, children: label }, value))) })] }));
}

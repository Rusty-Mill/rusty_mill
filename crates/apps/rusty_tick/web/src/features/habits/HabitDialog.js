import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { Dialog } from '@/components/Dialog';
import { SWATCHES } from '@/lib/colors';
import { Swatch } from '../lists/ListDialog';
import { usePrefs } from '../settings/prefs';
import { formatGoal, parseGoal, weekdayOrder } from './logic';
const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
export function HabitDialog({ open, onClose, habit, onSubmit }) {
    return (_jsx(Dialog, { open: open, onClose: onClose, title: habit ? 'Edit Habit' : 'Create Habit', width: 440, children: open && _jsx(HabitForm, { habit: habit, onCancel: onClose, onSubmit: onSubmit }, habit?.id ?? 'new') }));
}
function HabitForm({ habit, onCancel, onSubmit }) {
    const weekStart = usePrefs((s) => s.prefs.weekStart);
    const goal = parseGoal(habit?.goal ?? '');
    const f = habit?.frequency;
    const [name, setName] = useState(habit?.name ?? '');
    const [count, setCount] = useState(String(goal.count));
    const [unit, setUnit] = useState(goal.unit);
    const [kind, setKind] = useState(f?.kind ?? 'daily');
    const [days, setDays] = useState(f?.kind === 'weekdays' ? f.days : [1, 2, 3, 4, 5]);
    const [times, setTimes] = useState(String(f?.kind === 'perWeek' ? f.times : 3));
    const [color, setColor] = useState(habit?.color ?? SWATCHES[5]);
    const [reminder, setReminder] = useState(habit?.reminder ?? '');
    const goalCount = Math.min(9999, Math.max(1, Math.round(Number(count)) || 1));
    const timesNum = Math.min(7, Math.max(1, Math.round(Number(times)) || 1));
    const valid = name.trim().length > 0 && (kind !== 'weekdays' || days.length > 0);
    const submit = (e) => {
        e.preventDefault();
        if (!valid)
            return;
        const frequency = kind === 'daily' ? { kind } : kind === 'weekdays' ? { kind, days: [...days].sort((a, b) => a - b) } : { kind, times: timesNum };
        onSubmit({ name: name.trim(), color, goal: formatGoal(goalCount, unit), frequency, reminder: reminder || null });
    };
    const toggleDay = (d) => setDays((cur) => (cur.includes(d) ? cur.filter((x) => x !== d) : [...cur, d]));
    const input = 'h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary';
    const field = 'grid grid-cols-[104px_1fr] items-center gap-x-3';
    const label = 'text-base';
    const disabledSelect = `${input} w-full appearance-none bg-surface text-text opacity-70`;
    const today = new Date();
    return (_jsxs("form", { onSubmit: submit, className: "flex flex-col gap-3 overflow-y-auto px-6 pb-6 pt-2", children: [_jsxs("div", { className: field, children: [_jsx("span", { "aria-hidden": true, className: "flex h-10 w-10 items-center justify-center rounded-full text-base font-semibold text-white", style: { backgroundColor: color }, children: name.trim().charAt(0).toUpperCase() || '☺' }), _jsx("input", { "data-autofocus": true, "aria-label": "Name", value: name, maxLength: 100, onChange: (e) => setName(e.target.value), placeholder: "Habit name", className: input })] }), _jsxs("label", { className: field, children: [_jsx("span", { className: label, children: "Frequency" }), _jsxs("select", { value: kind, onChange: (e) => setKind(e.target.value), className: `${input} w-full`, children: [_jsx("option", { value: "daily", children: "Daily" }), _jsx("option", { value: "weekdays", children: "Specific days" }), _jsx("option", { value: "perWeek", children: "Times per week" })] })] }), kind === 'weekdays' && (_jsx("div", { className: "flex gap-1.5 pl-[116px]", role: "group", "aria-label": "Days of the week", children: weekdayOrder(weekStart).map((d) => (_jsx("button", { type: "button", "aria-pressed": days.includes(d), onClick: () => toggleDay(d), className: `h-8 w-9 rounded-row border text-s ${days.includes(d) ? 'border-primary bg-primary text-white' : 'border-line hover:bg-hover'}`, children: DAY_NAMES[d] }, d))) })), kind === 'perWeek' && (_jsxs("label", { className: "flex items-center gap-2 pl-[116px]", children: [_jsx("input", { type: "number", min: 1, max: 7, "aria-label": "Times per week", value: times, onChange: (e) => setTimes(e.target.value), className: `${input} w-20` }), _jsx("span", { className: "text-grey", children: "times per week" })] })), _jsxs("div", { className: field, role: "group", "aria-label": "Goal", children: [_jsx("span", { className: label, children: "Goal" }), _jsxs("div", { className: "flex items-center gap-2", children: [_jsx("input", { type: "number", min: 1, max: 9999, "aria-label": "Goal amount", value: count, onChange: (e) => setCount(e.target.value), className: `${input} w-20` }), _jsx("input", { "aria-label": "Goal unit", value: unit, maxLength: 30, onChange: (e) => setUnit(e.target.value), placeholder: "time", className: `${input} w-24 min-w-0` }), _jsx("span", { className: "text-grey", children: "per day" })] })] }), _jsxs("div", { className: field, children: [_jsx("span", { className: label, children: "Start Date" }), _jsx("select", { disabled: true, "aria-label": "Start Date", className: disabledSelect, children: _jsx("option", { children: today.toLocaleDateString('en-US', { month: 'long', day: 'numeric' }) }) })] }), _jsxs("div", { className: field, children: [_jsx("span", { className: label, children: "Goal Days" }), _jsx("select", { disabled: true, "aria-label": "Goal Days", className: disabledSelect, children: _jsx("option", { children: "Forever" }) })] }), _jsxs("div", { className: field, children: [_jsx("span", { className: label, children: "Section" }), _jsx("select", { disabled: true, "aria-label": "Section", className: disabledSelect, children: _jsx("option", { children: "Others" }) })] }), _jsxs("div", { className: field, role: "radiogroup", "aria-label": "Colour", children: [_jsx("span", { className: label, children: "Colour" }), _jsx("div", { className: "flex flex-wrap gap-2", children: SWATCHES.map((c) => (_jsx(Swatch, { label: c, selected: color === c, color: c, onSelect: () => setColor(c) }, c))) })] }), _jsxs("label", { className: field, children: [_jsx("span", { className: label, children: "Reminder" }), _jsxs("div", { className: "flex items-center gap-2", children: [_jsx("input", { type: "time", "aria-label": "Reminder", value: reminder, onChange: (e) => setReminder(e.target.value), className: `${input} w-32` }), reminder && (_jsx("button", { type: "button", onClick: () => setReminder(''), className: "h-8 rounded-row px-3 text-grey hover:bg-hover", children: "Clear" }))] })] }), _jsxs("label", { className: "flex items-center gap-2 border-t border-line pt-3 text-grey", children: [_jsx("input", { type: "checkbox", disabled: true }), " Auto pop-up of habit log"] }), _jsxs("div", { className: "flex justify-end gap-2 pt-1", children: [_jsx("button", { type: "button", onClick: onCancel, className: "h-9 rounded-row border border-line px-5 hover:bg-hover", children: "Cancel" }), _jsx("button", { type: "submit", disabled: !valid, className: "h-9 rounded-row bg-primary px-6 text-white disabled:opacity-40", children: "Save" })] })] }));
}

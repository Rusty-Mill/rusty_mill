import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useActions, useServices } from '@/app/services';
import { dayKey, startOfDay } from '@/lib/date';
import { useNow } from '@/lib/hooks';
import { byUpcoming, daysLeft, describe, fromForm } from './logic';
import { useCountdowns } from './store';
export function CountdownPage() {
    const { api } = useServices();
    const { notify } = useActions();
    const now = useNow(60_000);
    const items = useCountdowns((s) => s.items);
    const [name, setName] = useState('');
    const [day, setDay] = useState(dayKey(startOfDay(Date.now())));
    useEffect(() => {
        void useCountdowns.getState().load(api, notify);
    }, [api, notify]);
    const sorted = useMemo(() => byUpcoming(items, now), [items, now]);
    const body = fromForm(name, day);
    const submit = (e) => {
        e.preventDefault();
        if (!body)
            return;
        useCountdowns.getState().add(body);
        setName('');
    };
    return (_jsxs("main", { className: "flex min-w-0 flex-1 flex-col", children: [_jsx("header", { className: "flex h-14 shrink-0 items-center px-4", children: _jsx("h1", { className: "text-h1 font-semibold", children: "Countdown" }) }), _jsxs("form", { onSubmit: submit, className: "flex flex-wrap items-end gap-3 px-4 pb-4", children: [_jsxs("label", { className: "flex flex-col gap-1", children: [_jsx("span", { className: "text-s text-grey", children: "Name" }), _jsx("input", { value: name, maxLength: 200, onChange: (e) => setName(e.target.value), placeholder: "Exam, launch, birthday", className: "h-9 w-64 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), _jsxs("label", { className: "flex flex-col gap-1", children: [_jsx("span", { className: "text-s text-grey", children: "Date" }), _jsx("input", { type: "date", value: day, onChange: (e) => setDay(e.target.value), className: "h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), _jsx("button", { type: "submit", disabled: !body, className: "h-9 rounded-row bg-primary px-4 text-white disabled:opacity-40", children: "Add countdown" })] }), sorted.length === 0 ? (_jsx("p", { className: "px-4 text-grey", children: "No countdowns yet. Add an important date to count down to." })) : (_jsx("ul", { "aria-label": "Countdowns", className: "flex flex-col gap-2 overflow-y-auto px-4 pb-4", children: sorted.map((c) => {
                    const days = daysLeft(c.dateMs, now);
                    return (_jsxs("li", { className: "group flex items-center gap-3 rounded-row border border-line bg-surface px-4 py-3", children: [_jsxs("div", { className: "min-w-0 flex-1", children: [_jsx("div", { className: "truncate font-semibold", children: c.name }), _jsx("div", { className: "text-s text-grey", children: new Date(c.dateMs).toLocaleDateString(undefined, { weekday: 'short', year: 'numeric', month: 'short', day: 'numeric' }) })] }), _jsx("span", { className: `tabular-nums ${days < 0 ? 'text-grey' : days === 0 ? 'font-semibold text-primary' : ''}`, children: describe(days) }), _jsx("button", { type: "button", "aria-label": `Delete ${c.name}`, onClick: () => useCountdowns.getState().remove(c.id), className: "rounded-row p-1.5 text-grey opacity-0 hover:bg-selected focus-visible:opacity-100 group-hover:opacity-100", children: _jsx(Trash2, { size: 16, strokeWidth: 1.5 }) })] }, c.id));
                }) }))] }));
}

import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { ChevronDown, ChevronLeft, ChevronRight, LayoutGrid, MoreHorizontal, Plus } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useActions, useServices } from '@/app/services';
import { Confirm } from '@/components/Confirm';
import { HabitArt } from '@/components/Illustrations';
import { addDays, monthName, startOfDay } from '@/lib/date';
import { usePrefs } from '../settings/prefs';
import { HabitDialog } from './HabitDialog';
import { HabitRow, ROW_GRID } from './HabitRow';
import { weekDays } from './logic';
import { useHabits } from './store';
const LETTERS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];
/** The seven days ending today, each with an empty hatched circle: what a habit's check-in row will look like. */
function DayStrip({ today }) {
    const days = Array.from({ length: 7 }, (_, i) => addDays(today, i - 6));
    return (_jsx("ol", { "aria-label": "Last seven days", className: "mx-6 mt-2 grid grid-cols-7", children: days.map((d) => {
            const isToday = d === today;
            return (_jsxs("li", { className: "flex flex-col items-center gap-1 text-s", children: [_jsx("span", { className: "text-grey", children: new Date(d).toLocaleDateString('en-US', { weekday: 'short' }) }), _jsx("span", { className: isToday ? 'font-semibold text-primary' : 'font-semibold text-grey', children: new Date(d).getDate() }), _jsxs("svg", { width: "24", height: "24", viewBox: "0 0 24 24", "aria-hidden": true, children: [_jsx("defs", { children: _jsx("pattern", { id: "hatch", width: "4", height: "4", patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)", children: _jsx("line", { x1: "0", y1: "0", x2: "0", y2: "4", stroke: "rgb(var(--line))", strokeWidth: "2" }) }) }), _jsx("circle", { cx: "12", cy: "12", r: "10", fill: "url(#hatch)", stroke: "rgb(var(--line))" })] })] }, d));
        }) }));
}
export function HabitsPage() {
    const { api } = useServices();
    const { notify } = useActions();
    const weekStart = usePrefs((s) => s.prefs.weekStart);
    const habits = useHabits((s) => s.habits);
    const checkins = useHabits((s) => s.checkins);
    const [offset, setOffset] = useState(0); // weeks from the current one
    const [dialog, setDialog] = useState(null);
    const [deleting, setDeleting] = useState(null);
    const today = startOfDay(Date.now());
    useEffect(() => {
        void useHabits.getState().load(api, notify);
    }, [api, notify]);
    const days = useMemo(() => weekDays(addDays(today, offset * 7), weekStart), [today, offset, weekStart]);
    const visible = useMemo(() => habits.filter((h) => !h.archived), [habits]);
    const countsByHabit = useMemo(() => {
        const map = new Map();
        for (const c of Object.values(checkins)) {
            const days = map.get(c.habitId) ?? new Map();
            days.set(c.day, c.count);
            map.set(c.habitId, days);
        }
        return map;
    }, [checkins]);
    const first = new Date(days[0]);
    const last = new Date(days[6]);
    const range = `${monthName(first.getMonth())} ${first.getDate()} – ${first.getMonth() === last.getMonth() ? '' : `${monthName(last.getMonth())} `}${last.getDate()}, ${last.getFullYear()}`;
    const submit = (input) => {
        const target = dialog?.habit;
        if (target)
            useHabits.getState().update(target.id, input);
        else
            useHabits.getState().add(input);
        setDialog(null);
    };
    return (_jsxs("main", { className: "flex min-w-0 flex-1 flex-col", children: [_jsxs("header", { className: "flex h-14 shrink-0 items-center gap-3 px-4", children: [_jsxs("h1", { className: "flex items-center gap-1 text-h1 font-semibold", children: ["Habit ", _jsx(ChevronDown, { size: 16, "aria-hidden": true, className: "text-grey" })] }), _jsxs("div", { className: "ml-auto flex items-center gap-1", children: [_jsx("button", { type: "button", "aria-label": "Previous week", onClick: () => setOffset((o) => o - 1), className: iconBtn, children: _jsx(ChevronLeft, { size: 20, strokeWidth: 1.5 }) }), _jsx("span", { className: "min-w-[150px] text-center", "aria-live": "polite", children: range }), _jsx("button", { type: "button", "aria-label": "Next week", onClick: () => setOffset((o) => o + 1), className: iconBtn, children: _jsx(ChevronRight, { size: 20, strokeWidth: 1.5 }) }), _jsx("button", { type: "button", disabled: offset === 0, onClick: () => setOffset(0), className: "ml-1 h-8 rounded-row border border-line px-3 hover:bg-hover disabled:opacity-40", children: "This week" }), _jsx("button", { type: "button", disabled: true, "aria-label": "Habit sections", title: "Sections are not available yet", className: `${iconBtn} ml-2 opacity-50`, children: _jsx(LayoutGrid, { size: 20, strokeWidth: 1.5 }) }), visible.length > 0 && (_jsx("button", { type: "button", "aria-label": "Add habit", onClick: () => setDialog({ habit: null }), className: iconBtn, children: _jsx(Plus, { size: 20, strokeWidth: 1.5 }) })), _jsx("button", { type: "button", disabled: true, "aria-label": "More", className: `${iconBtn} opacity-50`, children: _jsx(MoreHorizontal, { size: 20, strokeWidth: 1.5 }) })] })] }), visible.length === 0 && _jsx(DayStrip, { today: today }), visible.length === 0 ? (_jsxs("div", { className: "flex flex-1 flex-col items-center justify-center gap-2 pb-16 text-center", children: [_jsx(HabitArt, {}), _jsx("h2", { className: "text-title font-semibold", children: "Develop a habit" }), _jsx("p", { className: "text-grey", children: "Every little bit counts" }), _jsx("button", { type: "button", onClick: () => setDialog({ habit: null }), className: "mt-3 h-9 rounded-row bg-primary px-5 text-white", children: "Add habit" })] })) : (_jsx("div", { className: "flex min-h-0 flex-1 flex-col overflow-x-auto", children: _jsxs("div", { className: "min-w-[640px]", children: [_jsxs("div", { className: `${ROW_GRID} border-b border-line py-2 text-s`, role: "row", "aria-label": "Days of the week", children: [_jsx("span", {}), days.map((d) => {
                                    const isToday = d === today;
                                    return (_jsxs("div", { className: `flex flex-col items-center leading-tight ${isToday ? 'text-primary' : 'text-grey'}`, "aria-current": isToday ? 'date' : undefined, children: [_jsx("span", { children: LETTERS[new Date(d).getDay()] }), _jsx("span", { className: `mt-0.5 flex h-6 w-6 items-center justify-center rounded-full text-base ${isToday ? 'bg-primary font-semibold text-white' : 'text-text'}`, children: new Date(d).getDate() })] }, d));
                                }), _jsx("span", { className: "text-center", children: "Streak" }), _jsx("span", {})] }), _jsx("ul", { className: "overflow-y-auto", "aria-label": "Habits", children: visible.map((h) => (_jsx(HabitRow, { habit: h, days: days, counts: countsByHabit.get(h.id) ?? EMPTY, today: today, weekStart: weekStart, onToggle: (day) => useHabits.getState().toggle(h.id, day), onEdit: () => setDialog({ habit: h }), onDelete: () => setDeleting(h) }, h.id))) })] }) })), _jsx(HabitDialog, { open: dialog !== null, habit: dialog?.habit ?? null, onClose: () => setDialog(null), onSubmit: submit }), _jsx(Confirm, { open: deleting !== null, title: "Delete habit?", message: `"${deleting?.name ?? ''}" and all of its check-ins will be deleted.`, confirmLabel: "Delete", danger: true, onCancel: () => setDeleting(null), onConfirm: () => {
                    if (deleting)
                        useHabits.getState().remove(deleting.id);
                    setDeleting(null);
                } })] }));
}
const EMPTY = new Map();
const iconBtn = 'flex h-8 w-8 items-center justify-center rounded-row text-grey hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary';

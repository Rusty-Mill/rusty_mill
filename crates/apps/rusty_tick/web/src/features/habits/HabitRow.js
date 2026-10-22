import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Check, Flame, MoreHorizontal } from 'lucide-react';
import { useState } from 'react';
import { Menu } from '@/components/Menu';
import { dayKey, monthName } from '@/lib/date';
import { computeStreak, frequencyLabel, isDue, parseGoal } from './logic';
export const ROW_GRID = 'grid grid-cols-[minmax(150px,1fr)_repeat(7,44px)_96px_36px] items-center';
const WEEKDAY_FULL = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
/** "Read, Tuesday Sep 29, not done", or "Water, Tuesday Sep 29, 2 of 8" for a goal of several. */
export function cellLabel(name, ms, count, goal = 1) {
    const d = new Date(ms);
    const status = goal > 1 ? `${count} of ${goal}` : count >= goal ? 'done' : 'not done';
    return `${name}, ${WEEKDAY_FULL[d.getDay()]} ${monthName(d.getMonth())} ${d.getDate()}, ${status}`;
}
export function HabitRow({ habit, days, counts, today, weekStart, onToggle, onEdit, onDelete }) {
    const [menuAnchor, setMenuAnchor] = useState(null);
    const [menuOpen, setMenuOpen] = useState(false);
    const goal = parseGoal(habit.goal).count;
    const checked = new Set([...counts].filter(([, n]) => n >= goal).map(([day]) => day)); // only days that met the goal keep a streak
    const streak = computeStreak(habit, checked, today, weekStart);
    const unit = streak.unit === 'day' ? 'day' : 'week';
    return (_jsxs("li", { className: `${ROW_GRID} group border-b border-line py-2 hover:bg-hover/60`, children: [_jsxs("div", { className: "flex min-w-0 items-center gap-3 pl-4", children: [_jsx("span", { "aria-hidden": true, className: "flex h-8 w-8 shrink-0 items-center justify-center rounded-full font-semibold text-white", style: { backgroundColor: habit.color }, children: [...habit.name][0]?.toUpperCase() }), _jsxs("div", { className: "min-w-0", children: [_jsx("div", { className: "truncate font-semibold", children: habit.name }), _jsxs("div", { className: "truncate text-s text-grey", children: [habit.goal, " \u00B7 ", frequencyLabel(habit.frequency)] })] })] }), days.map((d) => {
                const key = dayKey(d);
                const count = counts.get(key) ?? 0;
                const done = count >= goal;
                const partial = count > 0 && !done;
                const future = d > today;
                const due = isDue(habit, d);
                return (_jsx("div", { className: "flex justify-center", children: _jsxs("button", { type: "button", "aria-label": cellLabel(habit.name, d, count, goal), "aria-pressed": done, disabled: future, onClick: () => onToggle(key), style: done ? { backgroundColor: habit.color, borderColor: habit.color } : partial ? { borderColor: habit.color, color: habit.color } : undefined, className: `flex h-7 w-7 items-center justify-center rounded-full border-[1.5px] outline-hidden focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-surface disabled:cursor-default ${done ? 'text-white' : partial ? 'text-s font-semibold' : `border-dashed border-grey hover:bg-selected ${due ? 'opacity-90' : 'opacity-40'} ${future ? 'opacity-30' : ''}`}`, children: [done && _jsx(Check, { size: 16, strokeWidth: 2.5, "aria-hidden": true }), partial && _jsx("span", { "aria-hidden": true, children: count })] }) }, key));
            }), _jsxs("div", { className: `flex items-center justify-center gap-1 ${streak.current > 0 ? 'text-text' : 'text-grey'}`, title: `Best streak: ${streak.best} ${unit}${streak.best === 1 ? '' : 's'}`, children: [_jsx(Flame, { size: 16, strokeWidth: 1.5, className: streak.current > 0 ? 'text-[#faa700]' : '', "aria-hidden": true }), _jsxs("span", { className: "tabular-nums", children: [streak.current, " ", unit, streak.current === 1 ? '' : 's'] })] }), _jsxs("div", { className: "flex justify-center", children: [_jsx("button", { type: "button", ref: setMenuAnchor, "aria-label": `More actions for ${habit.name}`, "aria-haspopup": "menu", "aria-expanded": menuOpen, onClick: () => setMenuOpen((o) => !o), className: "rounded-row p-1.5 text-grey hover:bg-selected focus-visible:ring-2 focus-visible:ring-primary", children: _jsx(MoreHorizontal, { size: 20, strokeWidth: 1.5 }) }), _jsx(Menu, { anchor: menuAnchor, open: menuOpen, onClose: () => setMenuOpen(false), placement: "bottom-end", label: `${habit.name} actions`, items: [
                            { id: 'edit', label: 'Edit', onSelect: onEdit },
                            { id: 'delete', label: 'Delete', danger: true, onSelect: onDelete },
                        ] })] })] }));
}

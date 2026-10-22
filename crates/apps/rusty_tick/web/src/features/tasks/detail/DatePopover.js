import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Bell, CalendarRange, ChevronLeft, ChevronRight, Clock, Moon, Repeat, Sun, Sunrise } from 'lucide-react';
import { useMemo, useState } from 'react';
import { Popover } from '@/components/Popover';
import { addDays, addMonths, dayKey, monthGrid, monthName, startOfDay, weekdayName } from '@/lib/date';
import { describeRule } from '@/lib/recurrence';
import { usePrefs } from '../../settings/prefs';
import { CLEARED, presetOf, quickDates, reminderOptions, repeatRule, selectionOf, toFields, } from '../dateSelection';
const QUICK_ICONS = { today: Sun, tomorrow: Sunrise, nextWeek: CalendarRange, evening: Moon };
const pad = (n) => String(n).padStart(2, '0');
const timeValue = (t) => (t ? `${pad(t.h)}:${pad(t.m)}` : '');
const parseTime = (v) => {
    const m = /^(\d{2}):(\d{2})$/.exec(v);
    return m ? { h: Number(m[1]), m: Number(m[2]) } : null;
};
/**
 * The due-date popover: Date / Duration tabs, the four quick buttons, a month
 * grid, and rows for time, reminder and repeat. Nothing is saved until OK.
 */
export function DatePopover(props) {
    const { anchor, open, onClose } = props;
    return (_jsx(Popover, { anchor: anchor, open: open, onClose: onClose, placement: "bottom-start", ariaLabel: "Due date", role: "dialog", className: "w-[320px] p-3", children: open && _jsx(PopoverBody, { ...props }) }));
}
function PopoverBody({ onClose, fields, now, onApply }) {
    const prefs = usePrefs((s) => s.prefs);
    const [sel, setSel] = useState(() => selectionOf(fields));
    const [tab, setTab] = useState(fields.startMs !== null ? 'duration' : 'date');
    const [target, setTarget] = useState('due');
    const [shown, setShown] = useState(() => startOfDay(sel.day ?? now));
    const [customOpen, setCustomOpen] = useState(false);
    const [custom, setCustom] = useState({ freq: 'DAILY', interval: 2 });
    const today = startOfDay(now);
    const grid = useMemo(() => monthGrid(shown, prefs.weekStart), [shown, prefs.weekStart]);
    const weekdays = Array.from({ length: 7 }, (_, i) => weekdayName((prefs.weekStart + i) % 7));
    const shownMonth = new Date(shown).getMonth();
    const pickDay = (day) => {
        setSel((s) => {
            if (tab === 'duration' && target === 'start')
                return { ...s, startDay: day, day: s.day !== null && s.day < day ? day : s.day ?? day };
            // The due day: a start after it would be nonsense, so pull the start back.
            return { ...s, day, startDay: tab === 'duration' && s.startDay !== null && s.startDay > day ? day : s.startDay };
        });
    };
    const applyQuick = (q) => {
        setSel((s) => ({ ...s, day: q.day, time: q.time ?? s.time, reminder: q.time && !s.reminder ? prefs.defaultReminder : s.reminder }));
        setShown(startOfDay(q.day));
    };
    const setTime = (time) => setSel((s) => ({
        ...s,
        day: s.day ?? today,
        time,
        // A time with no reminder yet gets the user's default; removing the time drops a timed reminder.
        reminder: time ? (s.reminder || (s.time ? s.reminder : prefs.defaultReminder)) : s.time ? '' : s.reminder,
    }));
    const allDay = sel.time === null;
    const reminderChoices = reminderOptions(allDay);
    const reminderValid = reminderChoices.some((r) => r.value === sel.reminder);
    const preset = presetOf(sel.repeat);
    const dueAnchor = (sel.day ?? today) + (sel.time ? sel.time.h * 3_600_000 + sel.time.m * 60_000 : 0);
    const chooseRepeat = (p) => {
        if (p === 'custom')
            return setCustomOpen(true);
        setCustomOpen(false);
        setSel((s) => ({ ...s, day: s.day ?? today, repeat: repeatRule(p, dueAnchor) }));
    };
    const ok = () => {
        onApply(toFields(sel));
        onClose();
    };
    const clear = () => {
        onApply(CLEARED);
        onClose();
    };
    return (_jsxs("div", { className: "flex flex-col gap-2.5 text-base", children: [_jsx("div", { role: "tablist", "aria-label": "Date type", className: "grid grid-cols-2 gap-1 rounded-row bg-black/[.05] p-0.5", children: ['date', 'duration'].map((t) => (_jsx("button", { type: "button", role: "tab", "aria-selected": tab === t, onClick: () => setTab(t), className: `h-7 rounded-md text-base ${tab === t ? 'bg-surface font-semibold shadow-xs' : 'text-grey'}`, children: t === 'date' ? 'Date' : 'Duration' }, t))) }), tab === 'duration' && (_jsx("div", { role: "group", "aria-label": "Which date to set", className: "grid grid-cols-2 gap-1 text-s", children: ['start', 'due'].map((k) => {
                    const day = k === 'start' ? sel.startDay : sel.day;
                    return (_jsxs("button", { type: "button", "aria-pressed": target === k, onClick: () => setTarget(k), className: `rounded-row border px-2 py-1 text-left ${target === k ? 'border-primary text-primary' : 'border-line text-grey'}`, children: [_jsx("span", { className: "block text-[11px] uppercase", children: k === 'start' ? 'Start' : 'End' }), day === null ? 'Not set' : `${monthName(new Date(day).getMonth())} ${new Date(day).getDate()}`] }, k));
                }) })), _jsx("div", { className: "flex justify-between px-1", role: "group", "aria-label": "Quick dates", children: quickDates(now, prefs.weekStart).map((q) => {
                    const Icon = QUICK_ICONS[q.id];
                    return (_jsx("button", { type: "button", title: q.label, "aria-label": q.label, onClick: () => applyQuick(q), className: "flex h-9 w-9 items-center justify-center rounded-row text-grey hover:bg-hover hover:text-primary", children: _jsx(Icon, { size: 20, strokeWidth: 1.5 }) }, q.id));
                }) }), _jsxs("div", { className: "flex items-center justify-between px-1", children: [_jsxs("span", { className: "font-semibold", "aria-live": "polite", children: [monthName(shownMonth), " ", new Date(shown).getFullYear()] }), _jsxs("span", { className: "flex", children: [_jsx("button", { type: "button", "aria-label": "Previous month", onClick: () => setShown((s) => addMonths(s, -1)), className: "flex h-7 w-7 items-center justify-center rounded hover:bg-hover", children: _jsx(ChevronLeft, { size: 16 }) }), _jsx("button", { type: "button", "aria-label": "Next month", onClick: () => setShown((s) => addMonths(s, 1)), className: "flex h-7 w-7 items-center justify-center rounded hover:bg-hover", children: _jsx(ChevronRight, { size: 16 }) })] })] }), _jsx(DayGrid, { grid: grid, weekdays: weekdays, shownMonth: shownMonth, today: today, sel: sel, weekStart: prefs.weekStart, onPick: pickDay, onFocusMonth: setShown }), _jsx(Row, { icon: _jsx(Clock, { size: 18 }), label: "Time", children: _jsxs("div", { className: "flex items-center gap-1.5", children: [_jsx("input", { type: "time", "aria-label": "Time", value: timeValue(sel.time), onChange: (e) => setTime(parseTime(e.target.value)), className: "h-7 rounded border border-line bg-surface px-1.5 text-base outline-hidden focus:border-primary" }), sel.time && (_jsx("button", { type: "button", onClick: () => setTime(null), className: "text-s text-grey hover:text-text", children: "All day" }))] }) }), _jsx(Row, { icon: _jsx(Bell, { size: 18 }), label: "Reminder", children: _jsx("select", { "aria-label": "Reminder", value: reminderValid ? sel.reminder : '', disabled: sel.day === null, onChange: (e) => setSel((s) => ({ ...s, reminder: e.target.value })), className: "h-7 max-w-[170px] rounded border border-line bg-surface px-1 text-base outline-hidden focus:border-primary disabled:opacity-40", children: reminderChoices.map((r) => (_jsx("option", { value: r.value, children: r.label }, r.value))) }) }), _jsx(Row, { icon: _jsx(Repeat, { size: 18 }), label: "Repeat", children: _jsxs("select", { "aria-label": "Repeat", value: preset, disabled: sel.day === null, onChange: (e) => chooseRepeat(e.target.value), className: "h-7 max-w-[170px] rounded border border-line bg-surface px-1 text-base outline-hidden focus:border-primary disabled:opacity-40", children: [_jsx("option", { value: "none", children: "Does not repeat" }), _jsx("option", { value: "daily", children: "Daily" }), _jsx("option", { value: "weekly", children: "Weekly" }), _jsx("option", { value: "monthly", children: "Monthly" }), _jsx("option", { value: "yearly", children: "Yearly" }), _jsx("option", { value: "custom", children: preset === 'custom' && !customOpen ? describeRule(sel.repeat) : 'Custom…' })] }) }), (customOpen || preset === 'custom') && sel.day !== null && (_jsxs("div", { className: "flex items-center gap-1.5 pl-7 text-s", children: ["Every", _jsx("input", { type: "number", min: 1, max: 999, "aria-label": "Repeat every", value: custom.interval, onChange: (e) => {
                            const next = { ...custom, interval: Math.max(1, Number(e.target.value) || 1) };
                            setCustom(next);
                            setSel((s) => ({ ...s, repeat: repeatRule('custom', dueAnchor, next) }));
                        }, className: "h-7 w-14 rounded border border-line bg-surface px-1.5 outline-hidden focus:border-primary" }), _jsxs("select", { "aria-label": "Repeat unit", value: custom.freq, onChange: (e) => {
                            const next = { ...custom, freq: e.target.value };
                            setCustom(next);
                            setSel((s) => ({ ...s, repeat: repeatRule('custom', dueAnchor, next) }));
                        }, className: "h-7 rounded border border-line bg-surface px-1 outline-hidden focus:border-primary", children: [_jsx("option", { value: "DAILY", children: "days" }), _jsx("option", { value: "WEEKLY", children: "weeks" }), _jsx("option", { value: "MONTHLY", children: "months" }), _jsx("option", { value: "YEARLY", children: "years" })] })] })), _jsxs("div", { className: "flex items-center justify-between border-t border-line pt-2", children: [_jsx("button", { type: "button", onClick: clear, className: "h-8 rounded-row px-3 text-grey hover:bg-hover", children: "Clear" }), _jsx("button", { type: "button", onClick: ok, "data-autofocus": true, className: "h-8 rounded-row bg-primary px-5 text-white", children: "OK" })] })] }));
}
function Row({ icon, label, children }) {
    return (_jsxs("div", { className: "flex items-center justify-between gap-2 px-1", children: [_jsxs("span", { className: "flex items-center gap-2 text-grey", children: [icon, label] }), children] }));
}
/** The month grid. Arrow keys move by a day or a week, paging the month as needed. */
function DayGrid({ grid, weekdays, shownMonth, today, sel, onPick, onFocusMonth }) {
    const focusDay = sel.day ?? today;
    const move = (from, days, el) => {
        const to = startOfDay(addDays(from, days));
        if (new Date(to).getMonth() !== shownMonth)
            onFocusMonth(to);
        // Focus after the grid has re-rendered for the new month.
        requestAnimationFrame(() => el.closest('[role="grid"]')?.querySelector(`[data-day="${dayKey(to)}"]`)?.focus());
    };
    const rangeStart = sel.startDay;
    return (_jsxs("div", { role: "grid", "aria-label": "Calendar", children: [_jsx("div", { role: "row", className: "grid grid-cols-7 text-center text-s text-grey", children: weekdays.map((d) => (_jsx("span", { role: "columnheader", className: "py-1", children: d.slice(0, 2) }, d))) }), Array.from({ length: 6 }, (_, w) => (_jsx("div", { role: "row", className: "grid grid-cols-7", children: grid.slice(w * 7, w * 7 + 7).map((day) => {
                    const d = new Date(day);
                    const isSel = sel.day === day;
                    const isStart = rangeStart === day;
                    const inRange = rangeStart !== null && sel.day !== null && day > rangeStart && day < sel.day;
                    const isToday = day === today;
                    const outside = d.getMonth() !== shownMonth;
                    return (_jsx("button", { type: "button", role: "gridcell", "data-day": dayKey(day), tabIndex: day === focusDay ? 0 : -1, "aria-selected": isSel || isStart, "aria-label": `${weekdayName(d.getDay())}, ${monthName(d.getMonth())} ${d.getDate()}, ${d.getFullYear()}${isToday ? ', today' : ''}`, onClick: () => onPick(day), onKeyDown: (e) => {
                            const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
                            if (step !== undefined) {
                                e.preventDefault();
                                move(day, step, e.currentTarget);
                            }
                        }, className: `m-px flex h-8 items-center justify-center rounded-full text-base ${isSel || isStart ? 'bg-primary text-white' : inRange ? 'bg-primary/10' : isToday ? 'font-semibold text-primary ring-1 ring-primary/40' : outside ? 'text-grey/60 hover:bg-hover' : 'hover:bg-hover'}`, children: d.getDate() }, day));
                }) }, w)))] }));
}

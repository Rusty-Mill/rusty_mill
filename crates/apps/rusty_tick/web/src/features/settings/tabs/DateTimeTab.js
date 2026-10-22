import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { REMINDERS_TIMED } from '@/features/tasks/dateSelection';
import { usePrefs } from '../prefs';
import { PaneTitle, Row, Segmented, selectClass } from './controls';
export function DateTimeTab() {
    const prefs = usePrefs((s) => s.prefs);
    const update = usePrefs((s) => s.update);
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "Date & Time" }), _jsx(Row, { label: "Week starts on", hint: "Calendars, the Next 7 Days list and summaries start their week here.", children: _jsx(Segmented, { name: "week-start", label: "Week starts on", value: prefs.weekStart, onChange: (weekStart) => update({ weekStart }), options: [
                        { value: 0, label: 'Sunday' },
                        { value: 1, label: 'Monday' },
                        { value: 6, label: 'Saturday' },
                    ] }) }), _jsx(Row, { label: "Time format", children: _jsx(Segmented, { name: "time-format", label: "Time format", value: prefs.hour12 ? '12' : '24', onChange: (v) => update({ hour12: v === '12' }), options: [
                        { value: '12', label: '12-hour' },
                        { value: '24', label: '24-hour' },
                    ] }) }), _jsx(Row, { label: "Default reminder", hint: "Given to a task when you set a time on it.", htmlFor: "default-reminder", children: _jsx("select", { id: "default-reminder", value: prefs.defaultReminder, onChange: (e) => update({ defaultReminder: e.target.value }), className: selectClass, children: REMINDERS_TIMED.map((r) => (_jsx("option", { value: r.value, children: r.label }, r.value))) }) })] }));
}

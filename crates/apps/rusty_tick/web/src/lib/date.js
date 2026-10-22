/**
 * Local-time date helpers. Everything is Unix milliseconds in and out; all
 * "day" arithmetic goes through `Date` so DST shifts do not make a day 23 or
 * 25 hours long.
 */
export const MINUTE = 60_000;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;
export function startOfDay(ms) {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
}
/** `n` calendar days later at the same wall-clock time. */
export function addDays(ms, n) {
    const d = new Date(ms);
    d.setDate(d.getDate() + n);
    return d.getTime();
}
export function addMonths(ms, n) {
    const d = new Date(ms);
    const day = d.getDate();
    d.setDate(1);
    d.setMonth(d.getMonth() + n);
    const last = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
    d.setDate(Math.min(day, last)); // Jan 31 + 1 month is Feb 28/29, not March
    return d.getTime();
}
/** Whole calendar days from `a`'s day to `b`'s day. */
export function diffDays(a, b) {
    return Math.round((startOfDay(b) - startOfDay(a)) / DAY);
}
/** `ms` set to `hour:minute` on its own day. */
export function atTime(ms, hour, minute = 0) {
    const d = new Date(ms);
    d.setHours(hour, minute, 0, 0);
    return d.getTime();
}
/** Start of the week containing `ms`. */
export function startOfWeek(ms, weekStart) {
    const d = new Date(startOfDay(ms));
    const back = (d.getDay() - weekStart + 7) % 7;
    return addDays(d.getTime(), -back);
}
/** `YYYY-MM-DD` in local time. */
export function dayKey(ms) {
    const d = new Date(ms);
    const mm = String(d.getMonth() + 1).padStart(2, '0');
    const dd = String(d.getDate()).padStart(2, '0');
    return `${d.getFullYear()}-${mm}-${dd}`;
}
/** Local midnight of a `YYYY-MM-DD` key, or `null` if it is not a real date. */
export function parseDayKey(key) {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(key);
    if (!m)
        return null;
    const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const date = new Date(y, mo - 1, d);
    return date.getFullYear() === y && date.getMonth() === mo - 1 && date.getDate() === d ? date.getTime() : null;
}
/** Six weeks of days covering the month of `ms`, starting on `weekStart`. */
export function monthGrid(ms, weekStart) {
    const first = new Date(ms);
    first.setDate(1);
    const gridStart = startOfWeek(first.getTime(), weekStart);
    return Array.from({ length: 42 }, (_, i) => addDays(gridStart, i));
}
export function formatTime(ms, hour12) {
    return new Date(ms).toLocaleTimeString('en-US', { hour: hour12 ? 'numeric' : '2-digit', minute: '2-digit', hour12 });
}
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
export const monthName = (i) => MONTHS[i] ?? '';
export const weekdayName = (i) => WEEKDAYS[i] ?? '';
/**
 * `Today`, `Tomorrow`, `Yesterday`, a weekday within the week, `Sep 30`, or
 * `Sep 30, 2027`; a time is appended for timed tasks. Overdue is red: a timed
 * task once its time has passed, an all-day task once its day has.
 */
export function formatDue(task, now, hour12 = false) {
    if (task.dueMs === null)
        return null;
    const due = task.dueMs;
    const days = diffDays(now, due);
    const overdue = task.isAllDay ? days < 0 : due < now;
    const tone = overdue ? 'overdue' : days === 0 ? 'today' : 'future';
    let day;
    if (days === 0)
        day = 'Today';
    else if (days === 1)
        day = 'Tomorrow';
    else if (days === -1)
        day = 'Yesterday';
    else if (days > 1 && days < 7)
        day = weekdayName(new Date(due).getDay());
    else {
        const d = new Date(due);
        const base = `${monthName(d.getMonth())} ${d.getDate()}`;
        day = d.getFullYear() === new Date(now).getFullYear() ? base : `${base}, ${d.getFullYear()}`;
    }
    return { text: task.isAllDay ? day : `${day} ${formatTime(due, hour12)}`, tone };
}
/** The long form for the detail chip: `Tomorrow, Sep 30, 17:00`. */
export function formatDueLong(task, now, hour12 = false) {
    if (task.dueMs === null)
        return null;
    const d = new Date(task.dueMs);
    const days = diffDays(now, task.dueMs);
    const date = `${monthName(d.getMonth())} ${d.getDate()}`;
    const rel = days === 0 ? 'Today, ' : days === 1 ? 'Tomorrow, ' : days === -1 ? 'Yesterday, ' : '';
    const year = d.getFullYear() === new Date(now).getFullYear() ? '' : `, ${d.getFullYear()}`;
    return `${rel}${date}${year}${task.isAllDay ? '' : `, ${formatTime(task.dueMs, hour12)}`}`;
}

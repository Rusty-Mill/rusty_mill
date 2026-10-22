/** Pure focus-timer maths: no clock reads, no React. Callers pass `now`. */
import { addDays, dayKey, diffDays, monthName, startOfDay } from '@/lib/date';
import { isAmbient } from './ambient';
export const DEFAULT_SETTINGS = { focusMin: 25, shortMin: 5, longMin: 15, longEvery: 4, ambient: 'off' };
const clampInt = (v, min, max, fallback) => typeof v === 'number' && Number.isFinite(v) ? Math.min(max, Math.max(min, Math.round(v))) : fallback;
export function sanitizeSettings(input) {
    const o = (typeof input === 'object' && input !== null ? input : {});
    return {
        focusMin: clampInt(o.focusMin, 1, 180, DEFAULT_SETTINGS.focusMin),
        shortMin: clampInt(o.shortMin, 1, 60, DEFAULT_SETTINGS.shortMin),
        longMin: clampInt(o.longMin, 1, 120, DEFAULT_SETTINGS.longMin),
        longEvery: clampInt(o.longEvery, 2, 12, DEFAULT_SETTINGS.longEvery),
        ambient: isAmbient(o.ambient) ? o.ambient : DEFAULT_SETTINGS.ambient,
    };
}
/** A stored focus doc body is a record only if it has the record's fields (settings share the kind). */
export function asRecordBody(body) {
    if (typeof body !== 'object' || body === null)
        return null;
    const o = body;
    if (typeof o.startMs !== 'number' || typeof o.endMs !== 'number' || typeof o.durationSec !== 'number')
        return null;
    if (o.kind !== 'pomo' && o.kind !== 'stopwatch')
        return null;
    return {
        startMs: o.startMs,
        endMs: o.endMs,
        durationSec: o.durationSec,
        kind: o.kind,
        taskId: typeof o.taskId === 'string' ? o.taskId : null,
        taskTitle: typeof o.taskTitle === 'string' ? o.taskTitle : null,
        interruptions: typeof o.interruptions === 'number' && o.interruptions > 0 ? Math.floor(o.interruptions) : 0,
    };
}
/** `25:00`, or `1:05:00` once an hour is reached (stopwatch). */
export function formatClock(totalSec) {
    const s = Math.max(0, Math.floor(totalSec));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const pad = (n) => String(n).padStart(2, '0');
    return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`;
}
/** `1h 25m`, `25m`, `45s`; whole minutes once there is at least one. */
export function formatDuration(totalSec) {
    const s = Math.max(0, Math.round(totalSec));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    if (h > 0)
        return `${h}h ${m}m`;
    if (m > 0)
        return `${m}m`;
    return s > 0 ? `${s}s` : '0m';
}
export function computeStats(records, now) {
    const dayStart = startOfDay(now);
    const dayEnd = startOfDay(addDays(dayStart, 1));
    const out = { todayPomos: 0, todaySec: 0, totalPomos: 0, totalSec: 0 };
    for (const r of records) {
        const pomo = r.kind === 'pomo' ? 1 : 0;
        out.totalPomos += pomo;
        out.totalSec += r.durationSec;
        if (r.endMs >= dayStart && r.endMs < dayEnd) {
            out.todayPomos += pomo;
            out.todaySec += r.durationSec;
        }
    }
    return out;
}
/** Pomos finished today: decides when the next break is the long one. */
export const pomosToday = (records, now) => computeStats(records, now).todayPomos;
export function breakFor(pomoCount, s) {
    const long = pomoCount > 0 && pomoCount % s.longEvery === 0;
    return long ? { kind: 'long', sec: s.longMin * 60 } : { kind: 'short', sec: s.shortMin * 60 };
}
/** Newest first, grouped by the local day a session ended. */
export function groupByDay(records, now) {
    const sorted = [...records].sort((a, b) => b.endMs - a.endMs);
    const groups = [];
    for (const r of sorted) {
        const key = dayKey(r.endMs);
        const last = groups[groups.length - 1];
        if (last && last.key === key)
            last.records.push(r);
        else
            groups.push({ key, label: dayLabel(r.endMs, now), records: [r] });
    }
    return groups;
}
function dayLabel(ms, now) {
    const d = diffDays(now, ms);
    if (d === 0)
        return 'Today';
    if (d === -1)
        return 'Yesterday';
    const date = new Date(ms);
    const base = `${monthName(date.getMonth())} ${date.getDate()}`;
    return date.getFullYear() === new Date(now).getFullYear() ? base : `${base}, ${date.getFullYear()}`;
}
/** Milliseconds actually spent running: timestamps, not ticks, so a throttled tab stays right. */
export function elapsedMs(s, now) {
    return Math.max(0, (s.pausedAt ?? now) - s.startedMs - s.pausedTotalMs);
}
/** Seconds left on a countdown (never negative); `null` for a stopwatch. */
export function remainingSec(s, now) {
    return s.targetSec === null ? null : Math.max(0, Math.ceil((s.targetSec * 1000 - elapsedMs(s, now)) / 1000));
}
export const isFinished = (s, now) => s.targetSec !== null && elapsedMs(s, now) >= s.targetSec * 1000;
/** Fraction of the ring that is filled, 0..1. A stopwatch fills once a minute. */
export function ringProgress(s, mode, now) {
    if (!s)
        return mode === 'pomo' ? 1 : 0;
    if (s.targetSec === null)
        return (elapsedMs(s, now) % 60_000) / 60_000;
    return Math.min(1, Math.max(0, 1 - elapsedMs(s, now) / (s.targetSec * 1000)));
}

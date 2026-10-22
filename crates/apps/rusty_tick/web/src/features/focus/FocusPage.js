import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { ChevronRight, MoreHorizontal, Plus } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useActions, useData, useServices } from '@/app/services';
import { Confirm } from '@/components/Confirm';
import { usePrefs } from '../settings/prefs';
import { FocusRecords } from './FocusRecords';
import { EstimatesPanel } from '../estimates/EstimatesPanel';
import { actualPomos, estimateRows, latestByTask, MAX_ESTIMATE } from '../estimates/logic';
import { setEstimate, useEstimates } from '../estimates/store';
import { startAmbient } from './ambient';
import { Overview } from './Overview';
import { SettingsRow } from './SettingsRow';
import { TimerRing } from './TimerRing';
import { computeStats, elapsedMs, remainingSec, ringProgress } from './logic';
import { useFocus } from './store';
import { useNow } from './useNow';
export function FocusPage() {
    const { api } = useServices();
    const { notify } = useActions();
    const hour12 = usePrefs((s) => s.prefs.hour12);
    const { records, settings, mode, session, offer } = useFocus();
    const tasks = useData((s) => s.tasks);
    const estimates = useEstimates((s) => s.items);
    const [taskId, setTaskId] = useState('');
    const [confirmGiveUp, setConfirmGiveUp] = useState(false);
    const running = session !== null && session.pausedAt === null;
    const focusing = running && session.phase === 'focus';
    const now = useNow(running);
    const f = useFocus.getState;
    useEffect(() => {
        void f().load(api, notify);
        void useEstimates.getState().load(api, notify);
    }, [api, notify, f]);
    // Ambient sound plays while a focus session runs, not on pause or breaks.
    useEffect(() => (focusing && settings.ambient !== 'off' ? startAmbient(settings.ambient) : undefined), [focusing, settings.ambient]);
    // Backstop for the end-of-countdown timer in the store.
    useEffect(() => {
        f().checkFinished();
    }, [now, f]);
    const openTasks = useMemo(() => Object.values(tasks).filter((t) => t.deletedMs === null && t.status === 'open').sort((a, b) => a.title.localeCompare(b.title)), [tasks]);
    const stats = useMemo(() => computeStats(records, now), [records, now]);
    const rows = useMemo(() => estimateRows(estimates, records, tasks), [estimates, records, tasks]);
    const estimated = latestByTask(estimates).find((e) => e.taskId === taskId)?.pomos ?? 0;
    const isBreak = session?.phase === 'break';
    const shownMode = session ? session.mode : mode;
    const secs = session
        ? (remainingSec(session, now) ?? Math.floor(elapsedMs(session, now) / 1000))
        : shownMode === 'pomo'
            ? settings.focusMin * 60
            : 0;
    const caption = isBreak ? 'Break' : shownMode === 'pomo' ? 'Focus' : 'Stopwatch';
    const start = () => {
        const t = tasks[taskId];
        f().start(t ? { id: t.id, title: t.title } : null);
    };
    // Announced text only changes each minute (and on completion), so screen readers are not flooded.
    const remaining = session ? remainingSec(session, now) : null;
    const live = offer
        ? `Pomo complete. Time for a ${offer.kind} break.`
        : remaining !== null
            ? `${Math.ceil(remaining / 60)} minutes remaining`
            : '';
    return (_jsxs("main", { className: "flex min-w-0 flex-1", children: [_jsxs("section", { "aria-label": "Timer", className: "flex min-w-0 flex-1 flex-col items-center overflow-y-auto px-6 py-4", children: [_jsxs("header", { className: "grid w-full grid-cols-[1fr_auto_1fr] items-center", children: [_jsx("h1", { className: "text-title font-semibold", children: "Pomodoro" }), _jsx("div", { role: "tablist", "aria-label": "Timer mode", className: "flex rounded-full bg-side p-0.5", children: ['pomo', 'stopwatch'].map((m) => (_jsx("button", { type: "button", role: "tab", "aria-selected": shownMode === m, disabled: session !== null, onClick: () => f().setMode(m), className: `h-7 rounded-full px-5 disabled:cursor-default ${shownMode === m ? 'bg-surface font-semibold shadow-xs' : 'text-grey hover:text-text'}`, children: m === 'pomo' ? 'Pomo' : 'Stopwatch' }, m))) }), _jsxs("div", { className: "flex justify-end gap-1 text-grey", children: [_jsx("button", { type: "button", disabled: true, "aria-label": "Add a focus record", title: "Adding records by hand is not available yet", className: "flex h-8 w-8 items-center justify-center rounded-row opacity-50", children: _jsx(Plus, { size: 20 }) }), _jsx("button", { type: "button", disabled: true, "aria-label": "More", className: "flex h-8 w-8 items-center justify-center rounded-row opacity-50", children: _jsx(MoreHorizontal, { size: 20 }) })] })] }), _jsxs("label", { className: "relative mt-10 flex cursor-pointer items-center gap-1 text-grey", children: [_jsx("span", { children: tasks[taskId]?.title ?? (isBreak ? 'Break' : shownMode === 'pomo' ? 'Focus' : 'Stopwatch') }), _jsx(ChevronRight, { size: 14, "aria-hidden": true }), _jsxs("select", { "aria-label": "Task", value: taskId, disabled: session !== null, onChange: (e) => setTaskId(e.target.value), className: "absolute inset-0 cursor-pointer opacity-0", children: [_jsx("option", { value: "", children: "No task linked" }), openTasks.map((t) => (_jsx("option", { value: t.id, children: t.title }, t.id)))] })] }), taskId && !session && shownMode === 'pomo' && (_jsxs("div", { role: "group", "aria-label": "Pomo estimate", className: "mt-3 flex items-center gap-2 text-s text-grey", children: [_jsx("span", { children: "Estimated pomos" }), _jsx("button", { type: "button", "aria-label": "Fewer estimated pomos", disabled: estimated === 0, onClick: () => setEstimate(taskId, estimated - 1), className: stepper, children: "\u2212" }), _jsx("span", { "aria-live": "polite", className: "min-w-6 text-center tabular-nums text-text", children: estimated }), _jsx("button", { type: "button", "aria-label": "More estimated pomos", disabled: estimated >= MAX_ESTIMATE, onClick: () => setEstimate(taskId, estimated + 1), className: stepper, children: "+" }), _jsxs("span", { children: ["\u00B7 ", actualPomos(records, taskId), " done"] })] })), _jsx("div", { className: "mt-6", children: _jsx(TimerRing, { progress: offer ? 0 : ringProgress(session, shownMode, now), seconds: offer ? offer.sec : secs, caption: offer ? `${offer.kind === 'long' ? 'Long' : 'Short'} break` : caption, tone: isBreak || offer ? 'break' : 'focus' }) }), _jsx("p", { className: "sr-only", "aria-live": "polite", children: live }), session?.taskTitle && _jsxs("p", { className: "mt-2 text-s text-grey", children: ["Focusing on ", session.taskTitle] }), session && !isBreak && _jsxs("p", { className: "mt-1 text-s text-grey", "aria-live": "polite", children: ["Interruptions: ", session.interruptions] }), _jsx("div", { className: "mt-6 flex gap-3", children: offer ? (_jsxs(_Fragment, { children: [_jsx("button", { type: "button", onClick: () => f().startBreak(), className: primary, children: "Start break" }), _jsx("button", { type: "button", onClick: () => f().skipBreak(), className: secondary, children: "Skip" })] })) : !session ? (_jsx("button", { type: "button", onClick: start, className: primary, children: "Start" })) : (_jsxs(_Fragment, { children: [!isBreak && (_jsx("button", { type: "button", onClick: () => f().interrupt(), className: secondary, children: "Interrupted" })), _jsx("button", { type: "button", onClick: () => (session.pausedAt === null ? f().pause() : f().resume()), className: primary, children: session.pausedAt === null ? 'Pause' : 'Continue' }), isBreak ? (_jsx("button", { type: "button", onClick: () => f().stop(), className: secondary, children: "Skip break" })) : session.mode === 'pomo' ? (_jsx("button", { type: "button", onClick: () => setConfirmGiveUp(true), className: secondary, children: "Give up" })) : (_jsx("button", { type: "button", onClick: () => f().stop(), className: secondary, children: "Stop" }))] })) }), _jsx("div", { className: "mt-10", children: _jsx(SettingsRow, { settings: settings, onChange: (p) => f().updateSettings(p) }) })] }), _jsxs("aside", { "aria-label": "Focus overview", className: "flex w-[340px] shrink-0 flex-col border-l border-line max-[1000px]:w-[280px]", children: [_jsx(Overview, { stats: stats }), _jsx(EstimatesPanel, { rows: rows }), _jsxs("div", { className: "flex items-center justify-between px-5 pb-1 pt-5", children: [_jsx("h3", { className: "font-semibold", children: "Focus Record" }), _jsx(Plus, { size: 18, "aria-hidden": true, className: "text-grey opacity-50" })] }), _jsx(FocusRecords, { records: records, now: now, hour12: hour12, onDelete: (id) => f().deleteRecord(id) })] }), _jsx(Confirm, { open: confirmGiveUp, title: "Give up this pomo?", message: "The time so far will not be recorded.", confirmLabel: "Give up", danger: true, onCancel: () => setConfirmGiveUp(false), onConfirm: () => {
                    setConfirmGiveUp(false);
                    f().stop();
                } })] }));
}
const primary = 'h-12 min-w-[170px] rounded-full bg-primary px-8 font-semibold text-white outline-hidden focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-surface';
const stepper = 'flex h-6 w-6 items-center justify-center rounded-full border border-line text-text hover:bg-hover disabled:opacity-40';
const secondary = 'h-12 min-w-[120px] rounded-full border border-line px-6 hover:bg-hover';

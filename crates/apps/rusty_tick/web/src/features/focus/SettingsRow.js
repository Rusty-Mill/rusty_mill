import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useState } from 'react';
import { AMBIENT, isAmbient } from './ambient';
const FIELDS = [
    { key: 'focusMin', label: 'Focus', unit: 'min' },
    { key: 'shortMin', label: 'Short break', unit: 'min' },
    { key: 'longMin', label: 'Long break', unit: 'min' },
    { key: 'longEvery', label: 'Long break after', unit: 'pomos' },
];
export function SettingsRow({ settings, onChange }) {
    return (_jsxs("div", { className: "flex flex-wrap justify-center gap-x-5 gap-y-2", role: "group", "aria-label": "Timer settings", children: [FIELDS.map((f) => (_jsx(NumberField, { label: f.label, unit: f.unit, value: settings[f.key], onCommit: (v) => onChange({ [f.key]: v }) }, f.key))), _jsxs("label", { className: "flex items-center gap-1.5 text-s text-grey", children: ["Sound", _jsx("select", { value: settings.ambient, "aria-label": "Ambient sound", onChange: (e) => isAmbient(e.target.value) && onChange({ ambient: e.target.value }), className: "h-7 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary", children: AMBIENT.map((a) => (_jsx("option", { value: a.id, children: a.label }, a.id))) })] })] }));
}
/** Edits a draft and commits on blur or Enter, so typing "45" does not pass through a rejected "4". */
function NumberField({ label, unit, value, onCommit }) {
    const [draft, setDraft] = useState(String(value));
    useEffect(() => setDraft(String(value)), [value]);
    const commit = () => {
        const n = Number(draft);
        if (draft.trim() !== '' && Number.isFinite(n))
            onCommit(n);
        else
            setDraft(String(value));
    };
    return (_jsxs("label", { className: "flex items-center gap-1.5 text-s text-grey", children: [label, _jsx("input", { type: "number", inputMode: "numeric", min: 1, value: draft, "aria-label": `${label} (${unit})`, onChange: (e) => setDraft(e.target.value), onBlur: commit, onKeyDown: (e) => e.key === 'Enter' && commit(), className: "h-7 w-14 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary" })] }));
}

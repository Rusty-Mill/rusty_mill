import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useRef, useState } from 'react';
import { forgetUserData, getToken, identity } from '@/app/env';
import { useActions, useServices } from '@/app/services';
import { downloadFile } from '@/features/summary/download';
import { dayKey } from '@/lib/date';
import { Confirm } from '@/components/Confirm';
import { applyImport, readImport } from '@/features/import';
import { makeBackup, restoreBackup, wipeData } from '../backup';
import { page, resetDemoData, signOut } from '../session';
import { APP_NAME, APP_VERSION } from './AboutTab';
import { PaneTitle, Row } from './controls';
/** `••••••••abcd`: enough to recognise a token, not enough to use one. */
export function maskToken(token) {
    if (!token)
        return 'No token';
    return `${'•'.repeat(8)}${token.length > 12 ? token.slice(-4) : ''}`;
}
const button = 'h-8 rounded-row border border-line px-4 hover:bg-hover';
const card = 'flex flex-col rounded-[10px] bg-side px-4 py-1 [&>*+*]:border-t [&>*+*]:border-line';
const line = 'flex min-h-10 items-center justify-between gap-3 py-2 text-base';
const soon = 'text-primary/50';
export function AccountTab() {
    const { mode, api } = useServices();
    const { notify, refresh } = useActions();
    const who = mode === 'server' ? identity(getToken()) || 'Tick Local' : 'Demo';
    const backup = () => {
        makeBackup(api)
            .then((backup) => downloadFile(`tick-backup-${dayKey(Date.now())}.json`, 'application/json', JSON.stringify(backup, null, 2)))
            .catch(() => notify('error', 'Could not create a backup'));
    };
    const [confirming, setConfirming] = useState(false);
    const [wiping, setWiping] = useState(false);
    const file = useRef(null);
    const importFile = (f) => {
        if (!f)
            return;
        void f
            .text()
            .then((text) => restoreBackup(api, text))
            .then((n) => {
            notify('info', `Imported ${n.tasks} tasks, ${n.lists} lists, ${n.tags} tags and ${n.docs} other items`);
            void refresh();
        })
            .catch((e) => notify('error', e instanceof Error ? e.message : 'Could not import that backup'));
    };
    const other = useRef(null);
    const importOther = (f) => {
        if (!f)
            return;
        void f
            .text()
            .then(async (text) => {
            const read = readImport(text, f.name);
            const n = await applyImport(api, read);
            const notes = [n.existing && `${n.existing} already here`, n.resumed && `${n.resumed} finished from an earlier import`, read.skipped && `${read.skipped} rows skipped`, read.lossy && `${read.lossy} with a date or repeat kept in notes or dropped`].filter(Boolean);
            notify('info', `Imported ${n.tasks} tasks and ${n.lists} lists from ${read.source}${notes.length ? ` (${notes.join('; ')})` : ''}`);
            void refresh();
        })
            .catch((e) => notify('error', `Could not import ${f.name}: ${e instanceof Error ? e.message : String(e)}`));
    };
    const wipe = () => {
        setWiping(false);
        wipeData(api)
            .then(() => {
            forgetUserData();
            page.reload();
        })
            .catch(() => notify('error', 'Could not delete your data'));
    };
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "Account" }), _jsxs("div", { className: "mb-4 flex flex-col items-center gap-1", children: [_jsx("span", { "aria-hidden": true, className: "flex h-16 w-16 items-center justify-center rounded-full bg-selected text-title font-semibold text-grey", children: who.charAt(0).toUpperCase() }), _jsx("p", { className: "mt-1 font-semibold", children: who }), _jsx("p", { className: "text-s text-grey", children: mode === 'server' ? location.host : 'Stored in this browser' })] }), _jsxs("div", { className: `${card} mb-3`, children: [_jsxs("div", { className: line, children: [_jsx("span", { children: "Email" }), _jsx("span", { className: "text-grey", children: "Not used" })] }), _jsxs("div", { className: line, children: [_jsx("span", { children: "Password" }), _jsx("span", { className: soon, title: "Tick Local signs in with a token", children: "Change Password" })] }), _jsxs("div", { className: line, children: [_jsx("span", { children: "2-Step Verification" }), _jsx("span", { className: soon, title: "Not available", children: "Setting" })] })] }), _jsxs("div", { className: `${card} mb-3`, children: [_jsxs("div", { className: line, children: [_jsx("span", { children: "Login Devices" }), _jsx("span", { className: soon, title: "Not available", children: "Manage" })] }), _jsxs("div", { className: line, children: [_jsx("span", { children: "API Keys" }), _jsx("span", { className: soon, title: "Tokens are managed with the rusty_tick command line", children: "Manage" })] }), _jsxs("div", { className: line, children: [_jsx("span", { children: "Backup & Restore" }), _jsxs("span", { className: "flex gap-4", children: [_jsx("button", { type: "button", onClick: backup, className: "text-primary hover:underline", children: "Generate Backup" }), _jsx("button", { type: "button", onClick: () => file.current?.click(), className: "text-primary hover:underline", children: "Import Backups" }), _jsx("input", { ref: file, type: "file", accept: "application/json,.json", hidden: true, "aria-label": "Backup file", onChange: (e) => { importFile(e.target.files?.[0]); e.target.value = ''; } })] })] }), _jsxs("div", { className: line, children: [_jsx("span", { title: "TickTick backup, Todoist CSV, any CSV with a Title column (Apple Reminders via tools/export-apple-reminders.js), or .ics", children: "Import from Other Apps" }), _jsx("button", { type: "button", onClick: () => other.current?.click(), className: "text-primary hover:underline", children: "Import" }), _jsx("input", { ref: other, type: "file", accept: ".csv,.ics,text/csv,text/calendar", hidden: true, "aria-label": "File from another app", onChange: (e) => { importOther(e.target.files?.[0]); e.target.value = ''; } })] }), _jsxs("div", { className: line, children: [_jsx("span", { children: "Manage Account" }), _jsx("button", { type: "button", onClick: () => setWiping(true), className: "text-danger hover:underline", children: "Delete All Data" })] })] }), mode === 'server' ? (_jsxs(_Fragment, { children: [_jsx(Row, { label: "Connected to", hint: "The Tick Local server that stores your data.", children: _jsx("span", { className: "text-base", children: location.host }) }), _jsx(Row, { label: "Access token", hint: "Kept in this browser; it is never shown in full.", children: _jsx("code", { className: "text-base", children: maskToken(getToken()) }) }), _jsx(Row, { label: "Sign out", hint: "Forgets the token on this device. Your data stays on the server.", children: _jsx("button", { type: "button", onClick: signOut, className: button, children: "Sign out" }) })] })) : (_jsxs(_Fragment, { children: [_jsx("p", { className: "border-b border-line pb-4 text-base text-grey", children: "You are using the demo. Everything you create is stored in this browser only, and is lost if you clear the site's data." }), _jsx(Row, { label: "Reset sample data", hint: "Deletes your demo tasks and lists and starts again from the samples.", children: _jsx("button", { type: "button", onClick: () => setConfirming(true), className: `${button} text-danger`, children: "Reset sample data" }) }), _jsx(Confirm, { open: confirming, danger: true, title: "Reset sample data?", message: "This deletes every task and list in the demo and restores the sample data. It cannot be undone.", confirmLabel: "Reset", onCancel: () => setConfirming(false), onConfirm: resetDemoData })] })), _jsx(Confirm, { open: wiping, danger: true, title: "Delete all data?", message: "This permanently deletes every task, list, tag, habit and comment in this account. Your sign-in stays. It cannot be undone.", confirmLabel: "Delete", onCancel: () => setWiping(false), onConfirm: wipe }), _jsx(Row, { label: "Application", children: _jsxs("span", { className: "text-base text-grey", children: [APP_NAME, " ", APP_VERSION] }) })] }));
}

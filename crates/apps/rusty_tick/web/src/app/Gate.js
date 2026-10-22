import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { NoTasksArt } from '@/components/Illustrations';
import { adoptIdentity, clearToken, getToken, setMode, setToken } from './env';
const Card = ({ children }) => (_jsx("div", { className: "flex h-full items-center justify-center bg-side p-4", children: _jsxs("div", { className: "pop-shadow flex w-[420px] max-w-full flex-col gap-4 rounded-dialog bg-surface p-8", children: [_jsxs("div", { className: "flex items-center gap-3", children: [_jsx(NoTasksArt, {}), _jsx("h1", { className: "text-h1 font-semibold", children: "Tick Local" })] }), children] }) }));
/** First run: connect to a `rusty_tick` server, or try the app on sample data in this browser. */
export function Welcome({ onChoose }) {
    return (_jsxs(Card, { children: [_jsx("p", { className: "text-grey", children: "A private task manager. Your tasks live on your own server." }), _jsx("button", { type: "button", "data-autofocus": true, onClick: () => { setMode('server'); onChoose('server'); }, className: "h-10 rounded-row bg-primary text-white", children: "Connect to my server" }), _jsx("button", { type: "button", onClick: () => { setMode('demo'); onChoose('demo'); }, className: "h-10 rounded-row border border-line hover:bg-hover", children: "Try it with sample data" }), _jsx("p", { className: "text-s text-grey", children: "Sample data stays in this browser and is not sent anywhere." })] }));
}
/** Ask for the API token. It is kept for this tab only unless "remember" is ticked. */
export function TokenPrompt({ error, onSubmit, onDemo }) {
    const [token, setTokenText] = useState('');
    const [remember, setRemember] = useState(false);
    const submit = (e) => {
        e.preventDefault();
        if (!token.trim())
            return;
        setToken(token.trim(), remember);
        if (adoptIdentity(token.trim()))
            return location.reload(); // another user: start from their data, not the last one's
        onSubmit();
    };
    return (_jsxs(Card, { children: [_jsxs("form", { onSubmit: submit, className: "flex flex-col gap-3", children: [_jsxs("label", { className: "flex flex-col gap-1.5", children: [_jsx("span", { className: "text-s text-grey", children: "API token" }), _jsx("input", { "data-autofocus": true, type: "password", autoComplete: "off", value: token, onChange: (e) => setTokenText(e.target.value), placeholder: getToken() ? 'The saved token was not accepted' : 'The token the server was started with', className: "h-10 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), error && _jsx("p", { role: "alert", className: "text-s text-danger", children: error }), _jsxs("label", { className: "flex items-center gap-2 text-s text-grey", children: [_jsx("input", { type: "checkbox", checked: remember, onChange: (e) => setRemember(e.target.checked) }), "Remember on this device"] }), _jsx("button", { type: "submit", disabled: !token.trim(), className: "h-10 rounded-row bg-primary text-white disabled:opacity-40", children: "Connect" })] }), _jsx("button", { type: "button", onClick: () => { clearToken(); setMode('demo'); onDemo(); }, className: "text-s text-grey underline", children: "Use sample data instead" })] }));
}
export function Splash({ children }) {
    return _jsx("div", { className: "flex h-full items-center justify-center text-grey", children: children });
}

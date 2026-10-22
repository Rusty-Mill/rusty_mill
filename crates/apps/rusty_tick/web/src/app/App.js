import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useMemo, useState } from 'react';
import { RouterProvider } from 'react-router-dom';
import { usePrefs, applyTheme } from '@/features/settings/prefs';
import { getMode, getToken, setMode } from './env';
import { Splash, TokenPrompt, Welcome } from './Gate';
import { createRouter } from './router';
import { createServices, ServicesProvider, useData } from './services';
export function App() {
    const [mode, setModeState] = useState(getMode);
    // Bumped after a token is entered so the services (and their store) are built afresh.
    const [epoch, setEpoch] = useState(0);
    useEffect(() => applyTheme(usePrefs.getState().prefs.theme), []);
    if (!mode)
        return _jsx(Welcome, { onChoose: setModeState });
    if (mode === 'server' && !getToken()) {
        return _jsx(TokenPrompt, { onSubmit: () => setEpoch((e) => e + 1), onDemo: () => setModeState('demo') });
    }
    return _jsx(Connected, { mode: mode, onRetry: () => setEpoch((e) => e + 1), onDemo: () => { setMode('demo'); setModeState('demo'); } }, `${mode}:${epoch}`);
}
function Connected({ mode, onRetry, onDemo }) {
    const services = useMemo(() => createServices(mode), [mode]);
    useEffect(() => {
        const { store, api } = services;
        usePrefs.getState().bind(api);
        void store.getState().boot().then(() => usePrefs.getState().load(api));
        const stop = store.getState().startBackgroundSync();
        return () => {
            stop();
            usePrefs.getState().bind(null);
        };
    }, [services]);
    return (_jsx(ServicesProvider, { services: services, children: _jsx(Ready, { services: services, onRetry: onRetry, onDemo: onDemo }) }));
}
function Ready({ services, onRetry, onDemo }) {
    const status = useData((s) => s.status);
    const error = useData((s) => s.error);
    const router = useMemo(createRouter, []);
    if (status === 'loading')
        return _jsx(Splash, { children: "Loading\u2026" });
    if (status === 'unauthorized') {
        // The prompt stores the new token itself; retrying only rebuilds the services with it (clearing it here would discard it).
        return _jsx(TokenPrompt, { error: error ?? 'The server did not accept that token.', onSubmit: onRetry, onDemo: onDemo });
    }
    if (status === 'error') {
        return (_jsx(Splash, { children: _jsxs("div", { className: "flex flex-col items-center gap-3", children: [_jsxs("p", { role: "alert", children: ["Could not reach the server", error ? `: ${error}` : ''] }), _jsx("button", { type: "button", className: "h-9 rounded-row bg-primary px-4 text-white", onClick: () => services.store.getState().retryNow(), children: "Try again" }), _jsx("button", { type: "button", className: "text-s underline", onClick: onDemo, children: "Use sample data instead" })] }) }));
    }
    return _jsx(RouterProvider, { router: router });
}

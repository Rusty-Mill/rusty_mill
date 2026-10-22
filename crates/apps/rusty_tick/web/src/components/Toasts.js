import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { X } from 'lucide-react';
import { useEffect } from 'react';
import { useActions, useData } from '@/app/services';
const LIFETIME_MS = 6000;
/** Messages from failed saves and the like. Each disappears by itself, or when dismissed. */
export function Toasts() {
    const toasts = useData((s) => s.toasts);
    const { dismissToast } = useActions();
    useEffect(() => {
        const timers = toasts.map((t) => setTimeout(() => dismissToast(t.id), LIFETIME_MS));
        return () => timers.forEach(clearTimeout);
    }, [toasts, dismissToast]);
    if (toasts.length === 0)
        return null;
    return (_jsx("div", { role: "status", "aria-live": "polite", className: "fixed bottom-4 left-1/2 z-[70] flex -translate-x-1/2 flex-col gap-2", children: toasts.map((t) => (_jsxs("div", { className: `pop-shadow flex items-center gap-3 rounded-menu px-4 py-2.5 text-base text-white ${t.kind === 'error' ? 'bg-[#c0362c]' : 'bg-[#333]'}`, children: [_jsx("span", { children: t.message }), _jsx("button", { type: "button", "aria-label": "Dismiss", onClick: () => dismissToast(t.id), className: "rounded p-0.5 hover:bg-white/20", children: _jsx(X, { size: 14 }) })] }, t.id))) }));
}

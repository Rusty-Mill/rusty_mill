import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { X } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { createPortal } from 'react-dom';
const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';
/** A modal: traps Tab, closes on Escape or a press on the backdrop, and returns focus when it closes. */
export function Dialog({ open, onClose, title, labelledBy, width = 480, height, className = '', bare = false, children }) {
    const ref = useRef(null);
    const titleId = useId();
    const onCloseRef = useRef(onClose);
    onCloseRef.current = onClose;
    useEffect(() => {
        if (!open)
            return;
        const returnTo = document.activeElement;
        const el = ref.current;
        const first = el?.querySelector('[data-autofocus]') ?? el?.querySelector(FOCUSABLE) ?? el;
        first?.focus();
        const onKey = (e) => {
            if (e.key === 'Escape') {
                e.stopPropagation();
                return onCloseRef.current();
            }
            if (e.key !== 'Tab' || !el)
                return;
            const nodes = [...el.querySelectorAll(FOCUSABLE)].filter((n) => n.offsetParent !== null || n === document.activeElement);
            if (nodes.length === 0)
                return e.preventDefault();
            const firstNode = nodes[0];
            const lastNode = nodes[nodes.length - 1];
            if (e.shiftKey && document.activeElement === firstNode) {
                e.preventDefault();
                lastNode.focus();
            }
            else if (!e.shiftKey && document.activeElement === lastNode) {
                e.preventDefault();
                firstNode.focus();
            }
        };
        document.addEventListener('keydown', onKey);
        return () => {
            document.removeEventListener('keydown', onKey);
            if (returnTo && document.contains(returnTo))
                returnTo.focus();
        };
    }, [open]);
    if (!open)
        return null;
    return createPortal(_jsx("div", { className: "fixed inset-0 z-50 flex items-center justify-center bg-black/30", onMouseDown: (e) => {
            if (e.target === e.currentTarget)
                onClose();
        }, children: _jsxs("div", { ref: ref, role: "dialog", "aria-modal": "true", "aria-labelledby": labelledBy ?? (title ? titleId : undefined), tabIndex: -1, style: { width, height, maxWidth: 'calc(100vw - 32px)', maxHeight: 'calc(100vh - 32px)' }, className: `relative flex flex-col overflow-hidden rounded-dialog bg-surface text-text pop-shadow outline-hidden ${className}`, children: [title && (_jsx("h2", { id: titleId, className: "px-6 pb-2 pt-5 text-title font-semibold", children: title })), !bare && (_jsx("button", { type: "button", "aria-label": "Close", onClick: onClose, className: "absolute right-3 top-3 rounded-row p-1.5 text-grey hover:bg-hover", children: _jsx(X, { size: 18 }) })), children] }) }), document.body);
}

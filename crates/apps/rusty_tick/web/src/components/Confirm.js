import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Dialog } from './Dialog';
/** A yes/no question. Enter confirms; Escape or Cancel backs out. */
export function Confirm({ open, title, message, confirmLabel = 'OK', danger = false, onConfirm, onCancel }) {
    return (_jsxs(Dialog, { open: open, onClose: onCancel, title: title, width: 400, children: [_jsx("p", { className: "px-6 pb-4 text-base text-grey", children: message }), _jsxs("div", { className: "flex justify-end gap-2 px-6 pb-5", children: [_jsx("button", { type: "button", onClick: onCancel, className: "h-8 rounded-row px-4 hover:bg-hover", children: "Cancel" }), _jsx("button", { type: "button", "data-autofocus": true, onClick: onConfirm, className: `h-8 rounded-row px-4 text-white ${danger ? 'bg-danger' : 'bg-primary'}`, children: confirmLabel })] })] }));
}

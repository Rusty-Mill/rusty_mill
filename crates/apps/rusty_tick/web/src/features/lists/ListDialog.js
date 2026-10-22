import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { Dialog } from '@/components/Dialog';
import { SWATCHES } from '@/lib/colors';
export function ListDialog({ open, onClose, list, onSubmit }) {
    return (_jsx(Dialog, { open: open, onClose: onClose, title: list ? 'Edit List' : 'Add List', width: 400, children: open && _jsx(ListForm, { list: list, onCancel: onClose, onSubmit: onSubmit }, list?.id ?? 'new') }));
}
function ListForm({ list, onCancel, onSubmit }) {
    const [name, setName] = useState(list?.name ?? '');
    const [color, setColor] = useState(list?.color ?? null);
    const valid = name.trim().length > 0;
    const submit = (e) => {
        e.preventDefault();
        if (valid)
            onSubmit({ name: name.trim(), color });
    };
    return (_jsxs("form", { onSubmit: submit, className: "flex flex-col gap-4 px-6 pb-6 pt-2", children: [_jsxs("label", { className: "flex flex-col gap-1.5", children: [_jsx("span", { className: "text-s text-grey", children: "Name" }), _jsx("input", { "data-autofocus": true, value: name, maxLength: 200, onChange: (e) => setName(e.target.value), placeholder: "List name", className: "h-9 rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" })] }), _jsxs("fieldset", { className: "flex flex-col gap-1.5", children: [_jsx("legend", { className: "mb-1.5 text-s text-grey", children: "Colour" }), _jsxs("div", { className: "flex flex-wrap gap-2", role: "radiogroup", "aria-label": "Colour", children: [_jsx(Swatch, { label: "No colour", selected: color === null, color: null, onSelect: () => setColor(null) }), SWATCHES.map((c) => (_jsx(Swatch, { label: c, selected: color === c, color: c, onSelect: () => setColor(c) }, c)))] })] }), _jsxs("div", { className: "flex justify-end gap-2 pt-1", children: [_jsx("button", { type: "button", onClick: onCancel, className: "h-8 rounded-row px-4 hover:bg-hover", children: "Cancel" }), _jsx("button", { type: "submit", disabled: !valid, className: "h-8 rounded-row bg-primary px-4 text-white disabled:opacity-40", children: list ? 'Save' : 'Add' })] })] }));
}
export function Swatch({ color, selected, onSelect, label }) {
    return (_jsx("button", { type: "button", role: "radio", "aria-checked": selected, "aria-label": label, onClick: onSelect, style: { backgroundColor: color ?? 'transparent' }, className: `h-6 w-6 rounded-full border ${color ? 'border-transparent' : 'border-dashed border-grey'} ${selected ? 'ring-2 ring-primary ring-offset-2 ring-offset-surface' : ''}` }));
}

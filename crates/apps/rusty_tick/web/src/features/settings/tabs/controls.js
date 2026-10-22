import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** A pane's heading. Named by the tab, so the tabpanel and its heading agree. */
export function PaneTitle({ children }) {
    return _jsx("h3", { className: "mb-4 text-title font-semibold", children: children });
}
/** A setting: label and optional hint on the left, the control on the right. */
export function Row({ label, hint, htmlFor, children }) {
    return (_jsxs("div", { className: "flex items-center justify-between gap-6 border-b border-line py-3.5 last:border-b-0", children: [_jsxs("div", { className: "min-w-0", children: [_jsx("label", { htmlFor: htmlFor, className: "text-base", children: label }), hint && _jsx("p", { className: "text-s text-grey", children: hint })] }), _jsx("div", { className: "shrink-0", children: children })] }));
}
/** A row of radio buttons drawn as one control; native radios give the arrow keys and the labels. */
export function Segmented({ name, label, value, options, onChange }) {
    return (_jsx("div", { role: "radiogroup", "aria-label": label, className: "inline-flex rounded-row bg-selected p-0.5", children: options.map((o) => (_jsxs("label", { className: "cursor-pointer", children: [_jsx("input", { type: "radio", name: name, checked: value === o.value, onChange: () => onChange(o.value), className: "peer sr-only" }), _jsx("span", { className: "flex h-7 items-center rounded-[6px] px-3 text-base text-grey peer-checked:bg-surface peer-checked:font-semibold peer-checked:text-text peer-checked:shadow-xs peer-focus-visible:ring-2 peer-focus-visible:ring-primary", children: o.label })] }, String(o.value)))) }));
}
/** An on/off switch. */
export function Switch({ id, checked, onChange, label }) {
    return (_jsx("button", { id: id, type: "button", role: "switch", "aria-checked": checked, "aria-label": label, onClick: () => onChange(!checked), className: `relative h-6 w-10 rounded-full transition-colors ${checked ? 'bg-primary' : 'bg-selected'}`, children: _jsx("span", { className: `absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-all ${checked ? 'left-[18px]' : 'left-0.5'}` }) }));
}
export const selectClass = 'h-8 rounded-row border border-line bg-surface px-2 text-base text-text outline-hidden focus:border-primary';

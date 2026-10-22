import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { PaneTitle } from './controls';
/** Panes for things Tick Local does not have. Honest about it; nothing to buy or enable. */
export function PlaceholderTab({ title, note }) {
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: title }), _jsx("p", { className: "text-base text-grey", children: note ?? `${title} is not available in Tick Local.` })] }));
}

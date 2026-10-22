import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** Estimated against actual pomos, for every task that has an estimate. */
export function EstimatesPanel({ rows }) {
    if (rows.length === 0)
        return null;
    return (_jsxs("section", { "aria-label": "Estimates", className: "px-5 pt-5", children: [_jsx("h3", { className: "mb-2 font-semibold", children: "Estimates" }), _jsx("ul", { children: rows.map((r) => (_jsxs("li", { className: "flex items-center gap-3 py-1", children: [_jsx("span", { className: "min-w-0 flex-1 truncate", children: r.title }), _jsxs("span", { className: `tabular-nums ${r.actual > r.estimated ? 'text-danger' : 'text-grey'}`, children: [r.actual, " / ", r.estimated, " pomos"] })] }, r.taskId))) })] }));
}

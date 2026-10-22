import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { formatDuration } from './logic';
/** The four stat tiles. */
export function Overview({ stats }) {
    const tiles = [
        ["Today's Pomos", String(stats.todayPomos)],
        ["Today's Focus", formatDuration(stats.todaySec)],
        ['Total Pomos', String(stats.totalPomos)],
        ['Total Focus', formatDuration(stats.totalSec)],
    ];
    return (_jsxs("section", { "aria-label": "Overview", className: "px-5 pt-5", children: [_jsx("h3", { className: "mb-3 font-semibold", children: "Overview" }), _jsx("dl", { className: "grid grid-cols-2 gap-2", children: tiles.map(([label, value]) => (_jsxs("div", { className: "rounded-row bg-side px-3 py-2.5", children: [_jsx("dt", { className: "text-s text-grey", children: label }), _jsx("dd", { className: "mt-0.5 text-title font-semibold tabular-nums", children: value })] }, label))) })] }));
}

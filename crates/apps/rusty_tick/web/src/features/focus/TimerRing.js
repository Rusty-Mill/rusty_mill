import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { formatClock } from './logic';
const TICKS = 90;
/** The big dial: a ring of fine ticks (the filled share in colour) round the clock. The text is the accessible content; the SVG is decoration. */
export function TimerRing({ progress, seconds, caption, tone, size = 380 }) {
    const mid = size / 2;
    const outer = mid - 4;
    const inner = outer - 14;
    const filled = Math.round(TICKS * Math.min(1, Math.max(0, progress)));
    const color = tone === 'focus' ? 'rgb(var(--primary))' : 'rgb(var(--green))';
    return (_jsxs("div", { role: "timer", "aria-label": `${caption} timer`, className: "relative flex items-center justify-center", style: { width: size, height: size }, children: [_jsx("svg", { width: size, height: size, viewBox: `0 0 ${size} ${size}`, "aria-hidden": true, children: Array.from({ length: TICKS }, (_, i) => {
                    const a = (i / TICKS) * 2 * Math.PI - Math.PI / 2;
                    return (_jsx("line", { x1: mid + inner * Math.cos(a), y1: mid + inner * Math.sin(a), x2: mid + outer * Math.cos(a), y2: mid + outer * Math.sin(a), stroke: i < filled ? color : 'rgb(var(--line))', strokeWidth: "2", strokeLinecap: "round" }, i));
                }) }), _jsx("span", { className: "absolute text-[64px] font-light leading-none tabular-nums", children: formatClock(seconds) })] }));
}

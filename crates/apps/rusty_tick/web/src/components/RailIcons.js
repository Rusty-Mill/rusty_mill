import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
const KNOCK = 'rgb(var(--rail))';
function Svg({ children }) {
    return (_jsx("svg", { width: "22", height: "22", viewBox: "0 0 24 24", "aria-hidden": true, fill: "currentColor", children: children }));
}
export const TasksIcon = () => (_jsxs(Svg, { children: [_jsx("rect", { x: "3", y: "3", width: "18", height: "18", rx: "4.5" }), _jsx("path", { d: "M7.5 12.4l3 3 6-6.4", fill: "none", stroke: KNOCK, strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round" })] }));
export const CalendarIcon = () => (_jsxs(Svg, { children: [_jsx("rect", { x: "3", y: "4", width: "18", height: "17", rx: "4" }), [8, 12, 16].flatMap((x) => [10.5, 14.5, 18].map((y) => _jsx("circle", { cx: x, cy: y, r: "1", fill: KNOCK }, `${x}-${y}`))), _jsx("rect", { x: "7.5", y: "2", width: "1.8", height: "4", rx: ".9" }), _jsx("rect", { x: "14.7", y: "2", width: "1.8", height: "4", rx: ".9" })] }));
export const FocusIcon = () => (_jsxs(Svg, { children: [_jsx("circle", { cx: "12", cy: "12", r: "8", fill: "none", stroke: "currentColor", strokeWidth: "3" }), _jsx("circle", { cx: "12", cy: "12", r: "2.6" })] }));
export const HabitIcon = () => (_jsxs(Svg, { children: [_jsx("circle", { cx: "12", cy: "12", r: "9.5" }), _jsx("path", { d: "M12 6.5V12l3.5 2", fill: "none", stroke: KNOCK, strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round" })] }));
export const SearchIcon = () => (_jsxs(Svg, { children: [_jsx("circle", { cx: "10.5", cy: "10.5", r: "6.5", fill: "none", stroke: "currentColor", strokeWidth: "3" }), _jsx("path", { d: "M15.5 15.5L21 21", stroke: "currentColor", strokeWidth: "3", strokeLinecap: "round" })] }));

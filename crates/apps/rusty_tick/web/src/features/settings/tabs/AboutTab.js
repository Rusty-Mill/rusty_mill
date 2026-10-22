import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import pkg from '../../../../package.json';
import { PaneTitle } from './controls';
export const APP_NAME = 'Tick Local';
export const APP_VERSION = pkg.version;
export function AboutTab() {
    return (_jsxs(_Fragment, { children: [_jsx(PaneTitle, { children: "About" }), _jsx("p", { className: "text-base font-semibold", children: APP_NAME }), _jsxs("p", { className: "mb-3 text-s text-grey", children: ["Version ", APP_VERSION] }), _jsx("p", { className: "text-base text-grey", children: "A self-hosted task manager in the style of TickTick. Your lists, tasks, habits and settings are stored on your own server, or in this browser in demo mode." })] }));
}

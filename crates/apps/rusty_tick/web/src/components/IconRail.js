import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Bell, HelpCircle, Hourglass, RefreshCw, Sparkles, WifiOff } from 'lucide-react';
import { useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { calendarPath, HOME, PATHS, railSection } from '@/app/paths';
import { useActions, useData } from '@/app/services';
import { AccountMenu } from '@/features/settings/AccountMenu';
import { useUi } from '@/store/ui';
import { Popover } from './Popover';
import { CalendarIcon, FocusIcon, HabitIcon, SearchIcon, TasksIcon } from './RailIcons';
import { Tooltip } from './Tooltip';
const ICON = 20;
const STROKE = 1.5;
function RailButton({ label, active = false, onClick, children }) {
    return (_jsx(Tooltip, { label: label, children: _jsx("button", { type: "button", "aria-label": label, "aria-current": active ? 'page' : undefined, onClick: onClick, className: `flex h-[50px] w-[50px] items-center justify-center transition-colors duration-150 ${active ? 'text-primary' : 'text-grey hover:text-text'}`, children: children }) }));
}
/** The 50px column of section icons. The active one is filled blue with no background. */
export function IconRail() {
    const navigate = useNavigate();
    const { pathname } = useLocation();
    const section = railSection(pathname);
    const setSearchOpen = useUi((s) => s.setSearchOpen);
    const assistantOpen = useUi((s) => s.assistantOpen);
    const toggleAssistant = useUi((s) => s.toggleAssistant);
    const [account, setAccount] = useState(false);
    const avatarRef = useRef(null);
    return (_jsxs("nav", { "aria-label": "Sections", className: "flex h-full w-[50px] shrink-0 flex-col items-center bg-rail", children: [_jsx("button", { ref: avatarRef, type: "button", "aria-label": "Account", "aria-haspopup": "menu", onClick: () => setAccount((o) => !o), className: "mb-1 mt-3 flex h-8 w-8 items-center justify-center rounded-full bg-primary text-base font-semibold text-white", children: "T" }), _jsx(AccountMenu, { anchor: avatarRef.current, open: account, onClose: () => setAccount(false) }), _jsx(RailButton, { label: "Tasks", active: section === 'tasks', onClick: () => navigate(HOME), children: _jsx(TasksIcon, {}) }), _jsx(RailButton, { label: "Calendar", active: section === 'calendar', onClick: () => navigate(calendarPath()), children: _jsx(CalendarIcon, {}) }), _jsx(RailButton, { label: "Pomodoro", active: section === 'focus', onClick: () => navigate(PATHS.focus), children: _jsx(FocusIcon, {}) }), _jsx(RailButton, { label: "Habit Tracker", active: section === 'habit', onClick: () => navigate(PATHS.habit), children: _jsx(HabitIcon, {}) }), _jsx(RailButton, { label: "Countdown", active: section === 'countdown', onClick: () => navigate(PATHS.countdown), children: _jsx(Hourglass, { size: ICON, strokeWidth: STROKE }) }), _jsx(RailButton, { label: "Search", onClick: () => setSearchOpen(true), children: _jsx(SearchIcon, {}) }), _jsx(RailButton, { label: "Assistant", active: assistantOpen, onClick: toggleAssistant, children: _jsx(Sparkles, { size: ICON, strokeWidth: STROKE }) }), _jsxs("div", { className: "mt-auto flex flex-col items-center pb-2", children: [_jsx(SyncButton, {}), _jsx(NotificationsButton, {}), _jsx(HelpButton, {})] })] }));
}
/** Sync status: spins while edits are pending, shows a slash when the server is unreachable. */
function SyncButton() {
    const online = useData((s) => s.online);
    const pending = useData((s) => s.pending);
    const { retryNow, refresh } = useActions();
    const label = !online ? 'Offline: changes are saved here and will sync when the server is back' : pending > 0 ? `Syncing ${pending} change${pending === 1 ? '' : 's'}` : 'Synced';
    return (_jsx(RailButton, { label: label, onClick: () => (online ? void refresh() : retryNow()), children: online ? _jsx(RefreshCw, { size: ICON, strokeWidth: STROKE, className: pending > 0 ? 'animate-spin' : '' }) : _jsx(WifiOff, { size: ICON, strokeWidth: STROKE, className: "text-danger" }) }));
}
function NotificationsButton() {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    return (_jsxs("span", { ref: ref, children: [_jsx(RailButton, { label: "Notifications", onClick: () => setOpen((o) => !o), children: _jsx(Bell, { size: ICON, strokeWidth: STROKE }) }), _jsxs(Popover, { anchor: ref.current, open: open, onClose: () => setOpen(false), placement: "right-start", ariaLabel: "Notifications", className: "w-64 p-4", children: [_jsx("p", { className: "text-base font-semibold", children: "Notifications" }), _jsx("p", { className: "mt-2 text-s text-grey", children: "Nothing new. Reminders will appear here." })] })] }));
}
const SHORTCUTS = [
    ['N', 'Add a task'],
    ['J / K', 'Next / previous task'],
    ['Space', 'Complete or reopen'],
    ['Delete', 'Move to Trash'],
    ['1 2 3 4', 'Priority: high, medium, low, none'],
    ['Ctrl/⌘ K', 'Search'],
    ['Esc', 'Close'],
];
function HelpButton() {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    return (_jsxs("span", { ref: ref, children: [_jsx(RailButton, { label: "Help", onClick: () => setOpen((o) => !o), children: _jsx(HelpCircle, { size: ICON, strokeWidth: STROKE }) }), _jsxs(Popover, { anchor: ref.current, open: open, onClose: () => setOpen(false), placement: "right-start", ariaLabel: "Keyboard shortcuts", className: "w-72 p-4", children: [_jsx("p", { className: "mb-2 text-base font-semibold", children: "Keyboard shortcuts" }), _jsx("dl", { className: "grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-base", children: SHORTCUTS.map(([keys, what]) => (_jsxs("div", { className: "contents", children: [_jsx("dt", { className: "font-mono text-s text-grey", children: keys }), _jsx("dd", { children: what })] }, keys))) })] })] }));
}

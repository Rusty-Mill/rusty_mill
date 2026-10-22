import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { ArrowUpRight, Clock } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { taskPath } from '@/app/paths';
import { useActions, useData } from '@/app/services';
import { Popover } from '@/components/Popover';
import { TaskCheck } from '@/components/TaskCheck';
import { eventWhen, toEvent } from './layout';
/** The small card a click on a task opens: what it is, when, where, done, and a way into the detail pane. */
export function TaskPopover({ taskId, anchor, color, hour12, onClose }) {
    const navigate = useNavigate();
    const actions = useActions();
    const task = useData((s) => s.tasks[taskId]);
    const list = useData((s) => (task ? s.lists[task.listId] : undefined));
    const event = task ? toEvent(task, true) : null;
    if (!task)
        return null;
    return (_jsxs(Popover, { anchor: anchor, open: true, onClose: onClose, role: "dialog", ariaLabel: task.title, className: "w-[280px] p-3", children: [_jsxs("div", { className: "flex items-start gap-2.5", children: [_jsx("span", { className: "mt-[3px]", children: _jsx(TaskCheck, { checked: task.status !== 'open', priority: task.priority, label: `Complete ${task.title}`, onChange: () => {
                                void actions.toggleDone(task.id);
                                onClose(); // the bar it hangs off may vanish when the task leaves the calendar
                            } }) }), _jsx("h2", { className: `min-w-0 flex-1 break-words font-semibold ${task.status !== 'open' ? 'text-grey line-through' : ''}`, children: task.title })] }), _jsxs("dl", { className: "mt-2 space-y-1 pl-[26px] text-s text-grey", children: [event && (_jsxs("div", { className: "flex items-center gap-1.5", children: [_jsx("dt", { className: "sr-only", children: "Due" }), _jsx(Clock, { size: 14, strokeWidth: 1.5, "aria-hidden": true }), _jsx("dd", { children: eventWhen(event, hour12) })] })), _jsxs("div", { className: "flex items-center gap-1.5", children: [_jsx("dt", { className: "sr-only", children: "List" }), _jsx("span", { "aria-hidden": true, className: "h-2.5 w-2.5 rounded-full", style: { backgroundColor: color } }), _jsx("dd", { className: "truncate", children: list?.name ?? 'Inbox' })] })] }), _jsx("div", { className: "mt-3 flex justify-end", children: _jsxs("button", { type: "button", onClick: () => {
                        onClose();
                        navigate(taskPath({ kind: 'list', id: task.listId }, task.id));
                    }, className: "flex h-8 items-center gap-1 rounded-row border border-line px-3 outline-hidden hover:bg-hover focus-visible:ring-2 focus-visible:ring-primary", children: ["Open", _jsx(ArrowUpRight, { size: 16, strokeWidth: 1.5, "aria-hidden": true })] }) })] }));
}

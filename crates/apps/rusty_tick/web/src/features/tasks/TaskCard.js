import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useNavigate } from 'react-router-dom';
import { taskPath } from '@/app/paths';
import { TaskCheck } from '@/components/TaskCheck';
import { formatDue } from '@/lib/date';
import { usePrefs } from '../settings/prefs';
import { useTaskActions } from './useTaskActions';
const TONE = { overdue: 'text-danger', today: 'text-primary', future: 'text-grey' };
/** A draggable board card. `group` is the column it sits in; it rides along in the drag payload. */
export function TaskCard({ task, group, spec, now, selected }) {
    const navigate = useNavigate();
    const taskActions = useTaskActions();
    const hour12 = usePrefs((s) => s.prefs.hour12);
    const due = formatDue(task, now, hour12);
    return (_jsxs("li", { draggable: true, onDragStart: (e) => {
            e.dataTransfer.setData('application/x-tick-card', `${task.id}|${group}`);
            e.dataTransfer.effectAllowed = 'move';
        }, className: `cursor-grab rounded-row border border-line bg-surface p-2.5 ${selected ? 'ring-2 ring-primary/40' : ''}`, children: [_jsxs("div", { className: "flex items-start gap-2", children: [_jsx(TaskCheck, { checked: task.status !== 'open', priority: task.priority, label: task.title, onChange: () => taskActions.toggle(task.id) }), _jsx("button", { type: "button", onClick: () => navigate(taskPath(spec, task.id)), className: "min-w-0 flex-1 text-left text-base", children: _jsx("span", { className: "line-clamp-2", children: task.title }) })] }), due && _jsx("p", { className: `mt-1 pl-6 text-s ${TONE[due.tone]}`, children: due.text })] }));
}

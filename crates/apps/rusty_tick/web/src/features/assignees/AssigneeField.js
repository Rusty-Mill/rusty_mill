import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { UserRound } from 'lucide-react';
import { useEffect, useId, useMemo, useState } from 'react';
import { assigneeMap, knownNames, MAX_NAME } from './logic';
import { setAssignee, useAssignees } from './store';
/** The task's assignee: type a name (earlier names are suggested) and leave the field or press Enter to save; clear it to unassign. */
export function AssigneeField({ taskId, disabled }) {
    const items = useAssignees((s) => s.items);
    const current = useMemo(() => assigneeMap(items)[taskId] ?? '', [items, taskId]);
    const names = useMemo(() => knownNames(items), [items]);
    const [draft, setDraft] = useState(current);
    const listId = useId();
    useEffect(() => setDraft(current), [current]);
    const commit = () => {
        if (draft.trim() !== current)
            setAssignee(taskId, draft);
    };
    return (_jsxs("label", { className: "flex items-center gap-2 text-s text-grey", children: [_jsx(UserRound, { size: 14, "aria-hidden": true }), _jsx("span", { className: "sr-only", children: "Assignee" }), _jsx("input", { value: draft, disabled: disabled, maxLength: MAX_NAME, list: listId, placeholder: "Assignee", "aria-label": "Assignee", onChange: (e) => setDraft(e.target.value), onBlur: commit, onKeyDown: (e) => {
                    if (e.key === 'Enter')
                        e.currentTarget.blur();
                }, className: "h-7 min-w-0 flex-1 rounded-row bg-transparent px-1 text-base text-text outline-hidden placeholder:text-grey hover:bg-hover focus:bg-hover" }), _jsx("datalist", { id: listId, children: names.map((n) => (_jsx("option", { value: n }, n))) })] }));
}

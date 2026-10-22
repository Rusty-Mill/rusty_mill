import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/**
 * The assistant: a chat panel over `rusty_tick`'s `POST /api/agent`, the
 * first consumer of `@rusty-mill/agui-react`. The agent is deterministic and
 * store-free; this panel gives it its tools (`useAction`, see `tools.ts`),
 * run here against the same store the rest of the UI uses.
 */
import { Sparkles, X } from 'lucide-react';
import { useMemo, useState } from 'react';
import { AgentProvider, useAction, useAgent } from '@rusty-mill/agui-react';
import { contentText } from '@rusty-mill/agui-core';
import { getToken } from '@/app/env';
import { useActions, useData, useServices } from '@/app/services';
import { useView } from '@/app/useView';
import { assigneeMap } from '@/features/assignees/logic';
import { useAssignees } from '@/features/assignees/store';
import { useFilters } from '@/features/filters/store';
import { useUi } from '@/store/ui';
import { taskTools } from './tools';
export const AGENT_URL = '/api/agent';
/** Adds the bearer token at request time, so a token entered later is used. */
const authedFetch = (url, init) => {
    const headers = new Headers(init?.headers);
    const token = getToken();
    if (token)
        headers.set('Authorization', `Bearer ${token}`);
    return fetch(url, { ...init, headers });
};
/** Mounted once in the shell; shows nothing until opened from the rail. */
export function AssistantPanel({ fetchImpl = authedFetch }) {
    const open = useUi((s) => s.assistantOpen);
    const endpoint = useMemo(() => ({ url: AGENT_URL, fetch: fetchImpl }), [fetchImpl]);
    if (!open)
        return null;
    return (_jsx(AgentProvider, { endpoint: endpoint, children: _jsx(Panel, {}) }));
}
function Panel() {
    const close = useUi((s) => s.closeAssistant);
    const { mode } = useServices();
    const { messages, running, error, send } = useAgent();
    const [draft, setDraft] = useState('');
    useTaskTools();
    const submit = (e) => {
        e.preventDefault();
        const text = draft.trim();
        if (!text || running)
            return;
        setDraft('');
        void send(text);
    };
    return (_jsxs("aside", { "aria-label": "Assistant", className: "pop-shadow fixed bottom-4 right-4 z-[60] flex h-[420px] w-[340px] flex-col rounded-menu bg-surface text-base", children: [_jsxs("header", { className: "flex items-center gap-2 border-b border-line px-3 py-2", children: [_jsx(Sparkles, { size: 16, className: "text-primary" }), _jsx("span", { className: "font-semibold", children: "Assistant" }), _jsx("button", { type: "button", "aria-label": "Close assistant", onClick: close, className: "ml-auto rounded p-0.5 hover:bg-hover", children: _jsx(X, { size: 16 }) })] }), _jsxs("ol", { "aria-label": "Conversation", className: "flex flex-1 flex-col gap-2 overflow-y-auto px-3 py-2", children: [messages.length === 0 && _jsx("li", { className: "text-s text-grey", children: mode === 'demo' ? 'The assistant needs the server; sample data has none.' : 'Try “add buy milk”, “what’s due today?”, “find milk” or “complete buy milk”' }), messages.map((m) => (_jsx(Bubble, { message: m }, m.id))), running && _jsx("li", { className: "text-s text-grey", children: "\u2026" }), error && (_jsx("li", { role: "alert", className: "text-s text-danger", children: error }))] }), _jsx("form", { onSubmit: submit, className: "border-t border-line p-2", children: _jsx("input", { "aria-label": "Message the assistant", value: draft, onChange: (e) => setDraft(e.target.value), placeholder: "Ask the assistant", disabled: running, className: "h-9 w-full rounded-row border border-line bg-surface px-3 outline-hidden focus:border-primary" }) })] }));
}
function Bubble({ message }) {
    if (message.role === 'tool')
        return null;
    if (message.role !== 'user' && message.role !== 'assistant')
        return null;
    const text = contentText(message.content);
    if (!text)
        return null;
    const mine = message.role === 'user';
    return (_jsx("li", { className: `max-w-[85%] whitespace-pre-wrap rounded-menu px-3 py-1.5 ${mine ? 'self-end bg-primary text-white' : 'self-start bg-hover'}`, children: text }));
}
/** The agent's tools (`tools.ts`), bound to the store and the list that is open. */
function useTaskTools() {
    const { spec } = useView();
    const tasks = useData((s) => s.tasks);
    const lists = useData((s) => s.lists);
    const tags = useData((s) => s.tags);
    const inboxId = useData((s) => s.inboxId);
    const filters = useFilters((s) => s.items);
    const assigneeItems = useAssignees((s) => s.items);
    const actions = useActions();
    const [add, search, due, complete, reopen, update, remove, listLists] = taskTools({
        entities: { tasks: Object.values(tasks), lists: Object.values(lists), tags: Object.values(tags), inboxId, filters, assignees: assigneeMap(assigneeItems) },
        listId: spec?.kind === 'list' ? spec.id : inboxId,
        now: Date.now(),
        createTask: actions.createTask,
        toggleDone: actions.toggleDone,
        reopen: (id) => actions.updateTask(id, { status: 'open' }),
        trash: actions.trashTask,
        update: actions.updateTask,
        move: actions.moveTask,
    });
    useAction(add);
    useAction(search);
    useAction(due);
    useAction(complete);
    useAction(reopen);
    useAction(update);
    useAction(remove);
    useAction(listLists);
}

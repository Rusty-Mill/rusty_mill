import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { Plus } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useActions } from '@/app/services';
import { parseQuickAdd } from '@/lib/nlp/parse';
import { usePrefs } from '../settings/prefs';
import { useUi } from '@/store/ui';
import { buildQuickAdd, defaultListId } from './quickAddParse';
/** Tokens the parser recognised, drawn behind the (transparent) input text so they show as highlights. */
function Highlighted({ text, tokens }) {
    const parts = [];
    let at = 0;
    tokens.forEach((t, i) => {
        if (t.start > at)
            parts.push(text.slice(at, t.start));
        parts.push(_jsx("mark", { className: "rounded bg-primary/15 text-primary", children: text.slice(t.start, t.end) }, i));
        at = t.end;
    });
    parts.push(text.slice(at));
    return _jsx(_Fragment, { children: parts });
}
/** The 38px "Add task to …" box. Enter adds; date, priority, `#tag` and `~list` are read from the text. */
export function QuickAdd({ view, lists, inboxId, tasks, now }) {
    const [text, setText] = useState('');
    const [invalid, setInvalid] = useState(false);
    const input = useRef(null);
    const under = useRef(null);
    const actions = useActions();
    const defaultReminder = usePrefs((s) => s.prefs.defaultReminder);
    const focusTick = useUi((s) => s.quickAddFocus);
    useEffect(() => {
        if (focusTick > 0)
            input.current?.focus();
    }, [focusTick]);
    const target = lists.find((l) => l.id === defaultListId(view, inboxId));
    const parsed = useMemo(() => parseQuickAdd(text, now), [text, now]);
    const submit = () => {
        const result = buildQuickAdd(text, {
            now: Date.now(),
            lists,
            inboxId,
            view,
            siblings: (listId) => tasks.filter((t) => t.listId === listId),
            defaultReminder,
        });
        if (!result.ok) {
            setInvalid(true); // nothing left for a title
            return;
        }
        setText('');
        setInvalid(false);
        void actions.createTask(result.input).catch((e) => actions.notify('error', e instanceof Error ? e.message : String(e)));
    };
    const onKeyDown = (e) => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault();
            submit();
        }
        else if (e.key === 'Escape') {
            setText('');
            input.current?.blur();
        }
    };
    return (_jsxs("div", { className: `relative mx-4 mb-2 flex h-[38px] items-center rounded-row bg-black/[.04] focus-within:bg-surface focus-within:ring-1 ${invalid ? 'ring-danger' : 'focus-within:ring-primary/50'}`, children: [_jsx(Plus, { size: 18, className: "pointer-events-none absolute left-3 text-grey", "aria-hidden": true }), _jsx("div", { ref: under, "aria-hidden": true, className: "pointer-events-none absolute inset-y-0 left-10 right-3 flex items-center overflow-hidden whitespace-pre text-base", children: _jsx("span", { children: _jsx(Highlighted, { text: text, tokens: parsed.tokens }) }) }), _jsx("input", { ref: input, "aria-label": "Add task", value: text, onChange: (e) => {
                    setText(e.target.value);
                    setInvalid(false);
                }, onScroll: (e) => {
                    if (under.current)
                        under.current.scrollLeft = e.currentTarget.scrollLeft;
                }, onKeyDown: onKeyDown, placeholder: `Add task to "${target?.name ?? 'Inbox'}"`, autoComplete: "off", spellCheck: false, 
                // The typed text is drawn by the layer behind (so tokens can be highlighted); the caret stays visible.
                className: "h-full w-full bg-transparent pl-10 pr-3 text-base text-transparent caret-[rgb(var(--text))] outline-hidden placeholder:text-grey" })] }));
}

import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { TaskCheck } from '@/components/TaskCheck';
import { useDraft } from '@/lib/useDraft';
import { applyFormat, applySlash, detectSlash, matchCommands } from './editing';
import { parseBlocks, parseInline, toggleCheckbox } from './markdown';
function renderInline(nodes) {
    return nodes.map((n, i) => {
        switch (n.type) {
            case 'text':
                return n.text;
            case 'bold':
                return _jsx("strong", { children: renderInline(n.children) }, i);
            case 'italic':
                return _jsx("em", { children: renderInline(n.children) }, i);
            case 'strike':
                return _jsx("s", { children: renderInline(n.children) }, i);
            case 'code':
                return (_jsx("code", { className: "rounded bg-black/[.06] px-1 font-mono text-[13px]", children: n.text }, i));
            case 'link':
                return (_jsx("a", { href: n.href, target: "_blank", rel: "noopener noreferrer", onClick: (e) => e.stopPropagation(), className: "text-primary underline", children: renderInline(n.children) }, i));
        }
    });
}
/** A description box: rendered markdown until you click into it, then a plain textarea with `/` commands. */
export const NotesEditor = forwardRef(function NotesEditor({ taskId, notes, disabled = false, onSave }, ref) {
    const draft = useDraft(taskId, notes, onSave, 500);
    const [editing, setEditing] = useState(false);
    const [slash, setSlash] = useState(null);
    const [active, setActive] = useState(0);
    const area = useRef(null);
    const pending = useRef(null);
    const commands = slash ? matchCommands(slash.query) : [];
    // Grow with the text.
    useEffect(() => {
        const el = area.current;
        if (!el)
            return;
        el.style.height = 'auto';
        el.style.height = `${Math.max(el.scrollHeight, 96)}px`;
    }, [draft.value, editing]);
    // Restore a selection after a programmatic edit.
    useEffect(() => {
        const sel = pending.current;
        if (sel && area.current) {
            area.current.focus();
            area.current.setSelectionRange(sel.start, sel.end);
            pending.current = null;
        }
    });
    useImperativeHandle(ref, () => ({
        focus: () => setEditing(true),
        format: (kind) => {
            const el = area.current;
            const start = el?.selectionStart ?? draft.value.length;
            const end = el?.selectionEnd ?? draft.value.length;
            const edit = applyFormat(draft.value, start, end, kind);
            pending.current = { start: edit.start, end: edit.end };
            draft.set(edit.text);
            setEditing(true);
        },
    }));
    const track = (el) => {
        const s = detectSlash(el.value, el.selectionStart);
        setSlash(s);
        if (s)
            setActive(0);
    };
    const choose = (index) => {
        const el = area.current;
        if (!slash || !el)
            return;
        const cmd = commands[index];
        if (!cmd)
            return;
        const edit = applySlash(draft.value, slash, el.selectionStart, cmd);
        pending.current = { start: edit.start, end: edit.end };
        draft.set(edit.text);
        setSlash(null);
    };
    const onKeyDown = (e) => {
        if (slash && commands.length > 0) {
            if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                e.preventDefault();
                setActive((a) => (a + (e.key === 'ArrowDown' ? 1 : -1) + commands.length) % commands.length);
            }
            else if (e.key === 'Enter' || e.key === 'Tab') {
                e.preventDefault();
                choose(active);
            }
            else if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                setSlash(null);
            }
            return;
        }
        // Enter continues a list: "- item" + Enter starts "- ".
        if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
            const el = e.currentTarget;
            const before = el.value.slice(0, el.selectionStart);
            const line = before.slice(before.lastIndexOf('\n') + 1);
            const m = /^(\s*(?:[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+))(.*)$/.exec(line);
            if (m && el.selectionStart === el.selectionEnd) {
                e.preventDefault();
                const marker = m[1].replace(/\[[xX]\]/, '[ ]');
                // Enter on an empty list item ends the list instead.
                const start = el.selectionStart - line.length;
                const next = m[2] === '' ? el.value.slice(0, start) + el.value.slice(el.selectionStart) : `${el.value.slice(0, el.selectionStart)}\n${nextMarker(marker)}${el.value.slice(el.selectionEnd)}`;
                const pos = m[2] === '' ? start : el.selectionStart + 1 + nextMarker(marker).length;
                pending.current = { start: pos, end: pos };
                draft.set(next);
            }
        }
    };
    const blocks = parseBlocks(draft.value);
    const show = editing || draft.value === '';
    return (_jsx("div", { className: "relative", children: show ? (_jsxs(_Fragment, { children: [_jsx("textarea", { ref: area, "aria-label": "Description", value: draft.value, disabled: disabled, autoFocus: editing, maxLength: 100_000, placeholder: "Write something or type / for commands", onFocus: () => setEditing(true), onChange: (e) => {
                        draft.set(e.target.value);
                        track(e.target);
                    }, onSelect: (e) => track(e.currentTarget), onKeyDown: onKeyDown, onBlur: () => {
                        draft.flush();
                        setSlash(null);
                        setEditing(false);
                    }, className: "block w-full resize-none bg-transparent text-base leading-[22px] outline-hidden placeholder:text-grey", style: { minHeight: 96 } }), slash && commands.length > 0 && (_jsx("ul", { role: "listbox", "aria-label": "Commands", className: "pop-shadow absolute left-0 top-full z-30 mt-1 w-56 rounded-menu border border-line bg-surface py-1", children: commands.map((c, i) => (_jsx("li", { role: "option", "aria-selected": i === active, children: _jsx("button", { type: "button", tabIndex: -1, onMouseDown: (e) => { e.preventDefault(); choose(i); }, className: `flex h-8 w-full items-center px-3 text-left ${i === active ? 'bg-hover' : ''}`, children: c.label }) }, c.id))) }))] })) : (_jsx("div", { role: "group", "aria-label": "Description (press Enter to edit)", tabIndex: disabled ? -1 : 0, onClick: () => !disabled && setEditing(true), onKeyDown: (e) => {
                if (e.key === 'Enter' && e.target === e.currentTarget && !disabled)
                    setEditing(true);
            }, className: "min-h-[96px] cursor-text text-base leading-[22px]", children: blocks.map((b, i) => (_jsx(BlockView, { block: b, onToggle: (line) => { draft.set(toggleCheckbox(draft.value, line)); draft.flush(); }, disabled: disabled }, i))) })) }));
});
/** The marker for the line after `marker`: numbers count up, bullets repeat. */
function nextMarker(marker) {
    const m = /^(\s*)(\d+)([.)]\s+)$/.exec(marker);
    return m ? `${m[1]}${Number(m[2]) + 1}${m[3]}` : marker;
}
function BlockView({ block, onToggle, disabled }) {
    switch (block.type) {
        case 'blank':
            return _jsx("div", { className: "h-2" });
        case 'rule':
            return _jsx("hr", { className: "my-2 border-line" });
        case 'heading':
            return _jsx("p", { className: `font-semibold ${block.level === 1 ? 'text-title' : block.level === 2 ? 'text-[16px]' : 'text-base'}`, children: renderInline(block.children) });
        case 'bullet':
            return (_jsxs("p", { className: "flex gap-2 pl-1", children: [_jsx("span", { "aria-hidden": true, children: "\u2022" }), _jsx("span", { children: renderInline(block.children) })] }));
        case 'number':
            return (_jsxs("p", { className: "flex gap-2 pl-1", children: [_jsxs("span", { "aria-hidden": true, className: "min-w-[1.25em] text-right", children: [block.n, "."] }), _jsx("span", { children: renderInline(block.children) })] }));
        case 'check':
            return (_jsxs("p", { className: "flex items-center gap-2", children: [_jsx(TaskCheck, { size: 14, checked: block.checked, label: inlineText(block.children), onChange: () => !disabled && onToggle(block.line) }), _jsx("span", { className: block.checked ? 'text-grey line-through' : '', children: renderInline(block.children) })] }));
        case 'quote':
            return _jsx("p", { className: "border-l-2 border-line pl-3 text-grey", children: renderInline(block.children) });
        case 'paragraph':
            return _jsx("p", { children: renderInline(block.children) });
    }
}
function inlineText(nodes) {
    return nodes.map((n) => (n.type === 'text' || n.type === 'code' ? n.text : inlineText(n.children))).join('');
}
// Exported for tests.
export { parseInline };

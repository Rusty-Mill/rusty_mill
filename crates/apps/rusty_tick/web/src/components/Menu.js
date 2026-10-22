import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { ChevronRight, Check } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Popover } from './Popover';
const isItem = (e) => e !== 'separator' && !('custom' in e);
/** A keyboard-navigable menu: arrows, Home/End, Enter/Space, → opens a submenu, ← or Esc closes it. */
export function Menu({ anchor, open, onClose, items, placement = 'bottom-start', label, className = '', nested = false }) {
    const listRef = useRef(null);
    const [active, setActive] = useState(0);
    const [sub, setSub] = useState(null);
    const enabled = items.map((e, i) => (isItem(e) && !e.disabled ? i : -1)).filter((i) => i >= 0);
    useEffect(() => {
        if (!open)
            return;
        setActive(enabled[0] ?? 0);
        setSub(null);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open]);
    useEffect(() => {
        // While a submenu is open it owns the focus; when it closes, focus comes back here.
        if (open && !sub)
            listRef.current?.querySelector(`[data-index="${active}"]`)?.focus();
    }, [open, active, sub]);
    const move = (delta) => {
        const at = enabled.indexOf(active);
        setActive(enabled[(at + delta + enabled.length) % enabled.length] ?? active);
    };
    const choose = (item, el) => {
        if (item.disabled)
            return;
        if (item.submenu)
            return setSub({ id: item.id, anchor: el });
        item.onSelect?.();
        onClose();
    };
    const onKeyDown = (e) => {
        const item = items[active];
        switch (e.key) {
            case 'ArrowDown':
                e.preventDefault();
                return move(1);
            case 'ArrowUp':
                e.preventDefault();
                return move(-1);
            case 'Home':
                e.preventDefault();
                return setActive(enabled[0] ?? 0);
            case 'End':
                e.preventDefault();
                return setActive(enabled[enabled.length - 1] ?? 0);
            case 'ArrowRight':
                if (item && isItem(item) && item.submenu) {
                    e.preventDefault();
                    setSub({ id: item.id, anchor: e.target });
                }
                return;
            case 'ArrowLeft':
                if (nested) {
                    e.preventDefault();
                    onClose();
                }
                return;
            case 'Tab':
                return onClose();
        }
    };
    const subItem = sub ? items.find((e) => isItem(e) && e.id === sub.id) : undefined;
    return (_jsx(_Fragment, { children: _jsxs(Popover, { anchor: anchor, open: open, onClose: onClose, placement: placement, role: "menu", ariaLabel: label, className: `min-w-[180px] py-1.5 ${className}`, children: [_jsx("div", { ref: listRef, onKeyDown: onKeyDown, children: items.map((entry, i) => entry === 'separator' ? (_jsx("div", { role: "separator", className: "my-1 h-px bg-line" }, `sep-${i}`)) : 'custom' in entry ? (_jsx("div", { children: entry.custom }, entry.id)) : (_jsxs("button", { type: "button", role: entry.checked === undefined ? 'menuitem' : 'menuitemcheckbox', "aria-checked": entry.checked, "aria-haspopup": entry.submenu ? 'menu' : undefined, "aria-expanded": entry.submenu ? sub?.id === entry.id : undefined, "aria-disabled": entry.disabled || undefined, "data-index": i, tabIndex: i === active ? 0 : -1, onMouseEnter: () => setActive(i), onClick: (e) => choose(entry, e.currentTarget), className: `flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-base outline-hidden hover:bg-hover focus-visible:bg-hover ${entry.danger ? 'text-danger' : ''} ${entry.disabled ? 'cursor-default opacity-40' : ''}`, children: [entry.icon && _jsx("span", { className: "flex h-5 w-5 items-center justify-center text-grey", children: entry.icon }), _jsx("span", { className: "flex-1 truncate", children: entry.label }), entry.hint && _jsx("span", { className: "text-s text-grey", children: entry.hint }), entry.checked && _jsx(Check, { size: 16, className: "text-primary", "aria-hidden": true }), entry.submenu && _jsx(ChevronRight, { size: 16, className: "text-grey", "aria-hidden": true })] }, entry.id))) }), sub && subItem?.submenu && (_jsx(Menu, { nested: true, anchor: sub.anchor, open: true, onClose: () => setSub(null), items: wrapClose(subItem.submenu, onClose), placement: "right-start", label: subItem.label }))] }) }));
}
/** Picking something in a submenu closes the whole menu, not just the submenu. */
function wrapClose(entries, closeAll) {
    return entries.map((e) => (e === 'separator' || 'custom' in e ? e : { ...e, onSelect: e.onSelect ? () => { e.onSelect?.(); closeAll(); } : undefined }));
}

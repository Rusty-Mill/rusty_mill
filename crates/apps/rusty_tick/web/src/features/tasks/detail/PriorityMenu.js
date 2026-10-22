import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { Flag } from 'lucide-react';
import { useRef, useState } from 'react';
import { Menu } from '@/components/Menu';
import { PRIORITY_COLOR, PRIORITY_LABEL } from '@/components/TaskCheck';
/** The flag button (coloured by priority) and its four-choice menu. */
export function PriorityMenu({ value, onChange, disabled = false }) {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    return (_jsxs(_Fragment, { children: [_jsx("button", { ref: ref, type: "button", "aria-label": `Priority: ${PRIORITY_LABEL[value]}`, "aria-haspopup": "menu", "aria-expanded": open, disabled: disabled, onClick: () => setOpen((o) => !o), className: "flex h-8 w-8 items-center justify-center rounded-row hover:bg-hover disabled:opacity-40", children: _jsx(Flag, { size: 18, color: PRIORITY_COLOR[value], fill: value ? PRIORITY_COLOR[value] : 'none' }) }), _jsx(Menu, { anchor: ref.current, open: open, onClose: () => setOpen(false), label: "Priority", placement: "bottom-end", items: [5, 3, 1, 0].map((p) => ({
                    id: `p${p}`,
                    label: `${PRIORITY_LABEL[p]} priority`,
                    icon: _jsx(Flag, { size: 16, color: PRIORITY_COLOR[p], fill: p ? PRIORITY_COLOR[p] : 'none' }),
                    checked: value === p,
                    onSelect: () => onChange(p),
                })) })] }));
}

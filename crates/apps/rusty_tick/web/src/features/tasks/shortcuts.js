/** True when the key press is going into a text field (or a control that uses the same keys). */
export function isTyping(target) {
    if (!target)
        return false;
    const tag = target.tagName?.toUpperCase();
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT')
        return true;
    if (target.isContentEditable)
        return true;
    const role = target.getAttribute?.('role');
    return role === 'textbox' || role === 'combobox';
}
const PRIORITY_KEYS = { '1': 5, '2': 3, '3': 1, '4': 0 };
export function shortcutFor(e) {
    const mod = e.ctrlKey || e.metaKey;
    // Search works anywhere, even while typing.
    if (mod && !e.altKey && e.key.toLowerCase() === 'k')
        return { type: 'search' };
    if (e.key === 'Escape')
        return { type: 'close' };
    if (mod || e.altKey || isTyping(e.target))
        return null;
    switch (e.key) {
        case 'n':
        case 'N':
            return { type: 'newTask' };
        case 'j':
        case 'J':
            return { type: 'next' };
        case 'k':
        case 'K':
            return { type: 'previous' };
        case ' ':
            return { type: 'toggle' };
        case 'Delete':
        case 'Backspace':
            return { type: 'delete' };
    }
    const p = PRIORITY_KEYS[e.key];
    return p === undefined ? null : { type: 'priority', value: p };
}

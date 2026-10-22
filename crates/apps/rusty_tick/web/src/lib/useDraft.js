import { useCallback, useEffect, useRef, useState } from 'react';
/**
 * Local editing state over a server value, with debounced autosave.
 *
 * - A change to `key` (a different record) resets the draft, saving the old one first.
 * - A change to `serverValue` is adopted only while there is nothing unsaved, so
 *   an edit in progress is never overwritten by a refresh.
 * - Leaving (unmount) saves what is pending, so navigating away mid-typing loses nothing.
 */
export function useDraft(key, serverValue, save, delay = 500, equals = Object.is) {
    const [value, setValue] = useState(serverValue);
    const [dirty, setDirty] = useState(false);
    const latest = useRef({ value, dirty, save, key });
    latest.current = { value, dirty, save, key };
    // The `save` in force when the edit was made. Switching records swaps `save` for the new
    // record's before the pending edit is flushed; using that one would write the old record's
    // text into the new record.
    const pendingSave = useRef(save);
    const timer = useRef(undefined);
    const lastKey = useRef(key);
    const flush = useCallback(() => {
        clearTimeout(timer.current);
        const cur = latest.current;
        if (!cur.dirty)
            return;
        latest.current = { ...cur, dirty: false };
        setDirty(false);
        pendingSave.current(cur.value);
    }, []);
    const set = useCallback((next) => {
        pendingSave.current = latest.current.save;
        latest.current = { ...latest.current, value: next, dirty: true };
        setValue(next);
        setDirty(true);
        clearTimeout(timer.current);
        timer.current = setTimeout(flush, delay);
    }, [delay, flush]);
    // Another record: save the one we are leaving (with its own save function), then start over.
    useEffect(() => {
        if (lastKey.current === key)
            return;
        lastKey.current = key;
        setValue(serverValue);
        setDirty(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [key]);
    // The server moved on: take it, unless the user has typed something we have not saved.
    useEffect(() => {
        if (!latest.current.dirty && !equals(latest.current.value, serverValue))
            setValue(serverValue);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [serverValue]);
    useEffect(() => flush, [flush, key]);
    return { value, set, flush, dirty };
}

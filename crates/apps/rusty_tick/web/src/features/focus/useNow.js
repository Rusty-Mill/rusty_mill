import { useEffect, useState } from 'react';
/** `Date.now()` refreshed while `active`; also on returning to a throttled tab. */
export function useNow(active, intervalMs = 250) {
    const [now, setNow] = useState(() => Date.now());
    useEffect(() => {
        setNow(Date.now());
        if (!active)
            return;
        const tick = () => setNow(Date.now());
        const id = setInterval(tick, intervalMs);
        document.addEventListener('visibilitychange', tick);
        return () => {
            clearInterval(id);
            document.removeEventListener('visibilitychange', tick);
        };
    }, [active, intervalMs]);
    return now;
}

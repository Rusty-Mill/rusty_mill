/**
 * User preferences. Kept in localStorage for instant startup and stored on the
 * server as the one `prefs` document so they follow the user to another
 * browser. Anything that formats a date or starts a week reads from here.
 */
import { create } from 'zustand';
export const DEFAULT_PREFS = { weekStart: 1, hour12: false, theme: 'light', defaultReminder: 'TRIGGER:PT0S', notifications: false };
/** The single prefs document's id. */
export const PREFS_ID = '00000000-0000-7000-8000-0000000000aa';
const KEY = 'tick-local:prefs:v1';
function readLocal() {
    try {
        const raw = localStorage.getItem(KEY);
        if (raw)
            return sanitize(JSON.parse(raw));
    }
    catch {
        /* unreadable: use defaults */
    }
    return DEFAULT_PREFS;
}
/** Accept only well-formed values; anything else falls back to the default. */
export function sanitize(input) {
    const o = (typeof input === 'object' && input !== null ? input : {});
    return {
        weekStart: o.weekStart === 0 || o.weekStart === 1 || o.weekStart === 6 ? o.weekStart : DEFAULT_PREFS.weekStart,
        hour12: typeof o.hour12 === 'boolean' ? o.hour12 : DEFAULT_PREFS.hour12,
        theme: o.theme === 'light' || o.theme === 'dark' || o.theme === 'system' ? o.theme : DEFAULT_PREFS.theme,
        defaultReminder: typeof o.defaultReminder === 'string' && o.defaultReminder.length <= 64 ? o.defaultReminder : DEFAULT_PREFS.defaultReminder,
        notifications: typeof o.notifications === 'boolean' ? o.notifications : DEFAULT_PREFS.notifications,
    };
}
/** `light` or `dark` for a theme choice. */
export function resolveTheme(theme) {
    if (theme !== 'system')
        return theme;
    return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}
export function applyTheme(theme) {
    document.documentElement.dataset.theme = resolveTheme(theme);
}
export const usePrefs = create()((set, get) => {
    let api = null;
    let timer;
    const saveRemote = () => {
        clearTimeout(timer);
        timer = setTimeout(() => {
            void api?.putDoc('prefs', PREFS_ID, get().prefs).catch(() => undefined); // local copy is enough if this fails
        }, 600);
    };
    return {
        prefs: readLocal(),
        update(patch) {
            const prefs = sanitize({ ...get().prefs, ...patch });
            set({ prefs });
            try {
                localStorage.setItem(KEY, JSON.stringify(prefs));
            }
            catch {
                /* storage unavailable */
            }
            applyTheme(prefs.theme);
            saveRemote();
        },
        async load(client) {
            try {
                const docs = await client.listDocs('prefs');
                const remote = docs.find((d) => d.id === PREFS_ID);
                if (remote) {
                    const prefs = sanitize(remote.body);
                    set({ prefs });
                    try {
                        localStorage.setItem(KEY, JSON.stringify(prefs));
                    }
                    catch {
                        /* storage unavailable */
                    }
                    applyTheme(prefs.theme);
                }
            }
            catch {
                /* offline or refused: keep the local copy */
            }
        },
        bind(client) {
            api = client;
        },
    };
});

export const HOME = '/q/all/tasks';
const SMART = { all: { kind: 'all' }, today: { kind: 'today' }, week: { kind: 'week' } };
/** The view a route's params name, or `null` if they name nothing valid. */
export function parseView(params, inboxId) {
    if (params.smart !== undefined)
        return SMART[params.smart] ?? null;
    if (params.listId !== undefined)
        return params.listId === 'inbox' || params.listId === inboxId ? { kind: 'inbox' } : { kind: 'list', id: params.listId };
    if (params.tag !== undefined)
        return { kind: 'tag', name: params.tag };
    if (params.filterId !== undefined)
        return { kind: 'filter', id: params.filterId };
    return null;
}
/** The URL of a view (without a selected task). */
export function viewPath(spec) {
    switch (spec.kind) {
        case 'all':
            return '/q/all/tasks';
        case 'today':
            return '/q/today/tasks';
        case 'week':
            return '/q/week/tasks';
        case 'inbox':
            return '/p/inbox/tasks';
        case 'list':
            return `/p/${spec.id}/tasks`;
        case 'tag':
            return `/t/${encodeURIComponent(spec.name)}/tasks`;
        case 'filter':
            return `/f/${spec.id}/tasks`;
    }
}
/** The URL of a view with `taskId` open in the detail pane. */
export const taskPath = (spec, taskId) => `${viewPath(spec)}/${taskId}`;
export const CALENDAR_MODES = ['m', 'w', 'd', '3', 't', 'a'];
export const calendarPath = (mode = 'm') => `/c/all/calendar/${mode}`;
export const PATHS = {
    completed: '/q/all/completed',
    trash: '/q/all/trash',
    summary: '/q/all/summary',
    habit: '/q/all/habit',
    countdown: '/countdown',
    focus: '/focus',
};
/** Settings is a modal over whatever route is open: `?modalType=settings&tabs=account`. */
export const SETTINGS_TABS = ['account', 'premium', 'features', 'smart-list', 'notifications', 'date-time', 'appearance', 'ai', 'more', 'integrations', 'collaborate', 'shortcuts', 'about'];
export function settingsHref(tab = 'account') {
    return `?modalType=settings&tabs=${tab}`;
}
export function railSection(path) {
    if (path.startsWith('/c/'))
        return 'calendar';
    if (path.startsWith('/focus'))
        return 'focus';
    if (path.startsWith(PATHS.habit))
        return 'habit';
    if (path.startsWith(PATHS.countdown))
        return 'countdown';
    return 'tasks';
}

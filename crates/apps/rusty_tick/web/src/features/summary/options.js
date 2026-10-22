import { dayKey } from '@/lib/date';
import { RANGE_KEYS } from './range';
export const TEMPLATES = ['daily', 'weekly', 'simple'];
export const TEMPLATE_LABELS = { daily: 'Daily report', weekly: 'Weekly report', simple: 'Simple list' };
export function defaultOptions(now) {
    const today = dayKey(now);
    return {
        template: 'weekly',
        range: 'thisWeek',
        custom: { from: today, to: today },
        listIds: [],
        status: 'all',
        priorities: [],
        tags: [],
        groupBy: 'none',
        showList: true,
        showDue: true,
        showPriority: false,
        showTags: false,
        showCompletedTime: false,
    };
}
const isRecord = (x) => typeof x === 'object' && x !== null;
const strings = (x) => (Array.isArray(x) ? x.filter((s) => typeof s === 'string').slice(0, 200) : []);
const DAYKEY = /^\d{4}-\d{2}-\d{2}$/;
/** Accept only well-formed values from a stored document; anything else falls back to the default. */
export function sanitizeOptions(input, now) {
    const d = defaultOptions(now);
    if (!isRecord(input))
        return d;
    const bool = (k) => (typeof input[k] === 'boolean' ? input[k] : d[k]);
    const custom = isRecord(input.custom) ? input.custom : {};
    return {
        template: TEMPLATES.find((t) => t === input.template) ?? d.template,
        range: RANGE_KEYS.find((r) => r === input.range) ?? d.range,
        custom: {
            from: typeof custom.from === 'string' && DAYKEY.test(custom.from) ? custom.from : d.custom.from,
            to: typeof custom.to === 'string' && DAYKEY.test(custom.to) ? custom.to : d.custom.to,
        },
        listIds: strings(input.listIds),
        status: input.status === 'completed' || input.status === 'open' || input.status === 'all' ? input.status : d.status,
        priorities: Array.isArray(input.priorities) ? input.priorities.filter((p) => p === 0 || p === 1 || p === 3 || p === 5) : [],
        tags: strings(input.tags),
        groupBy: input.groupBy === 'list' ? 'list' : 'none',
        showList: bool('showList'),
        showDue: bool('showDue'),
        showPriority: bool('showPriority'),
        showTags: bool('showTags'),
        showCompletedTime: bool('showCompletedTime'),
    };
}

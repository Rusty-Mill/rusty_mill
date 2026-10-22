/**
 * A small, safe markdown subset for task descriptions. Parsing produces plain
 * data (blocks of inline nodes) that React renders as elements, so no user text
 * is ever injected as HTML. Links are limited to http(s) and mailto.
 *
 * Blocks: `#`..`###` headings, `- ` bullets, `1. ` numbers, `- [ ]`/`- [x]`
 * checkboxes, `> ` quotes, `---` rules, paragraphs.
 * Inline: **bold**, *italic*, ~~strike~~, `code`, [text](url).
 */
const SAFE_URL = /^(https?:\/\/|mailto:)/i;
const PATTERNS = [
    { type: 'code', re: /`([^`\n]+)`/ },
    { type: 'link', re: /\[([^\]\n]+)\]\(([^)\s]+)\)/ },
    // Lazy, and not followed by another `*`, so `**bold *and italic***` keeps the italic inside the bold.
    { type: 'bold', re: /\*\*(.+?)\*\*(?!\*)/ },
    { type: 'strike', re: /~~([^~\n]+)~~/ },
    { type: 'italic', re: /\*([^*\n]+)\*/ },
];
/** Parse inline markup, earliest match first; unmatched markers stay as text. */
export function parseInline(text) {
    let best = null;
    for (const { type, re } of PATTERNS) {
        const m = re.exec(text);
        if (!m || (best && m.index >= best.at))
            continue;
        let node;
        if (type === 'code')
            node = { type: 'code', text: m[1] };
        else if (type === 'link') {
            const href = m[2];
            // An unsafe scheme (javascript:, data:) is not a link: keep the whole thing as text.
            node = SAFE_URL.test(href) ? { type: 'link', href, children: parseInline(m[1]) } : { type: 'text', text: m[0] };
        }
        else
            node = { type, children: parseInline(m[1]) };
        best = { at: m.index, len: m[0].length, node };
    }
    if (!best)
        return text ? [{ type: 'text', text }] : [];
    const before = text.slice(0, best.at);
    return mergeText([...(before ? [{ type: 'text', text: before }] : []), best.node, ...parseInline(text.slice(best.at + best.len))]);
}
/** Join neighbouring text nodes, so a rejected link and the text around it read as one run. */
function mergeText(nodes) {
    const out = [];
    for (const n of nodes) {
        const last = out[out.length - 1];
        if (n.type === 'text' && last?.type === 'text')
            out[out.length - 1] = { type: 'text', text: last.text + n.text };
        else
            out.push(n);
    }
    return out;
}
export function parseBlocks(text) {
    return text.split('\n').map((line, i) => {
        if (line.trim() === '')
            return { type: 'blank' };
        if (/^\s*(---|\*\*\*|___)\s*$/.test(line))
            return { type: 'rule' };
        let m = /^(#{1,3})\s+(.*)$/.exec(line);
        if (m)
            return { type: 'heading', level: m[1].length, children: parseInline(m[2]) };
        m = /^\s*[-*+]\s+\[([ xX])\]\s?(.*)$/.exec(line);
        if (m)
            return { type: 'check', checked: m[1] !== ' ', line: i, children: parseInline(m[2]) };
        m = /^\s*[-*+]\s+(.*)$/.exec(line);
        if (m)
            return { type: 'bullet', children: parseInline(m[1]) };
        m = /^\s*(\d+)[.)]\s+(.*)$/.exec(line);
        if (m)
            return { type: 'number', n: Number(m[1]), children: parseInline(m[2]) };
        m = /^>\s?(.*)$/.exec(line);
        if (m)
            return { type: 'quote', children: parseInline(m[1]) };
        return { type: 'paragraph', children: parseInline(line) };
    });
}
/** Flip the checkbox on `line` of `text`; anything that is not a checkbox line is left alone. */
export function toggleCheckbox(text, line) {
    const lines = text.split('\n');
    const cur = lines[line];
    if (cur === undefined)
        return text;
    const next = cur.replace(/^(\s*[-*+]\s+\[)([ xX])(\])/, (_m, a, box, c) => `${a}${box === ' ' ? 'x' : ' '}${c}`);
    if (next === cur)
        return text;
    lines[line] = next;
    return lines.join('\n');
}

import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { ChevronDown, Copy, Sparkles } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useActions, useData, useServices } from '@/app/services';
import { Menu } from '@/components/Menu';
import { dayKey } from '@/lib/date';
import { usePrefs } from '@/features/settings/prefs';
import { SAVE_FORMATS, downloadFile } from './download';
import { isEmptyContent, toHtmlDocument, toMarkdown, toPlainText } from './exporters';
import { renderSummaryHtml } from './render';
import { RichEditor } from './RichEditor';
import { SummaryFilters } from './SummaryFilters';
import { SummaryToolbar } from './SummaryToolbar';
import { buildSummary } from './summary';
import { useSummaryOptions } from './useSummaryOptions';
/** `#q/all/summary`: an editable report of finished and due tasks, saved as a file or copied. */
export function SummaryPage() {
    const { api } = useServices();
    const { notify } = useActions();
    const tasks = useData((s) => s.tasks);
    const lists = useData((s) => s.lists);
    const tags = useData((s) => s.tags);
    const weekStart = usePrefs((s) => s.prefs.weekStart);
    const hour12 = usePrefs((s) => s.prefs.hour12);
    const { options, loaded, update } = useSummaryOptions(api);
    const editor = useRef(null);
    const saveBtn = useRef(null);
    const [saveOpen, setSaveOpen] = useState(false);
    const listArray = useMemo(() => Object.values(lists).sort((a, b) => a.sortOrder - b.sortOrder), [lists]);
    const tagArray = useMemo(() => Object.values(tags).sort((a, b) => a.sortOrder - b.sortOrder || a.label.localeCompare(b.label)), [tags]);
    const generate = (undoable) => {
        const doc = buildSummary(options, { tasks: Object.values(tasks), lists: listArray, now: Date.now(), weekStart, hour12 });
        editor.current?.setHtml(renderSummaryHtml(doc), undoable);
    };
    // Fill an empty page once the remembered options are in, so it never opens blank.
    const first = useRef(true);
    useEffect(() => {
        if (!loaded || !first.current)
            return;
        first.current = false;
        const root = editor.current?.root();
        if (root && isEmptyContent(root))
            generate(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [loaded]);
    const content = () => {
        const root = editor.current?.root() ?? null;
        if (!root || isEmptyContent(root)) {
            notify('info', 'Nothing to save yet. Generate a summary first.');
            return null;
        }
        return root;
    };
    const copy = async () => {
        const root = content();
        if (!root)
            return;
        try {
            await navigator.clipboard.writeText(toPlainText(root));
            notify('info', 'Copied');
        }
        catch {
            notify('error', 'Could not copy to the clipboard');
        }
    };
    const save = (format) => {
        const root = content();
        if (!root)
            return;
        const body = format === 'md' ? toMarkdown(root) : format === 'txt' ? toPlainText(root) : toHtmlDocument(root, 'Summary');
        downloadFile(`summary-${dayKey(Date.now())}.${format}`, SAVE_FORMATS[format].mime, body);
    };
    const saveItems = Object.keys(SAVE_FORMATS).map((f) => ({ id: f, label: SAVE_FORMATS[f].label, onSelect: () => save(f) }));
    return (_jsxs("main", { className: "flex min-w-0 flex-1 overflow-hidden", children: [_jsxs("section", { "aria-label": "Summary editor", className: "flex min-w-0 flex-1 flex-col", children: [_jsx("header", { className: "px-4 pb-2 pt-4", children: _jsx("h1", { className: "text-h1 font-semibold", children: "Summary" }) }), _jsxs("div", { className: "mx-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-[10px] border border-line", children: [_jsx(SummaryToolbar, { editor: editor }), _jsx(RichEditor, { ref: editor })] }), _jsxs("footer", { className: "flex justify-end gap-2 px-4 py-3", children: [_jsxs("button", { ref: saveBtn, type: "button", "aria-haspopup": "menu", "aria-expanded": saveOpen, onClick: () => setSaveOpen((o) => !o), className: "flex h-8 items-center gap-1.5 rounded-row px-3 hover:bg-hover", children: ["Save as ", _jsx(ChevronDown, { size: 14, className: "text-grey", "aria-hidden": true })] }), _jsxs("button", { type: "button", onClick: () => void copy(), className: "flex h-8 items-center gap-1.5 rounded-row px-3 hover:bg-hover", children: [_jsx(Copy, { size: 14, className: "text-grey", "aria-hidden": true }), " Copy"] }), _jsx(Menu, { anchor: saveBtn.current, open: saveOpen, onClose: () => setSaveOpen(false), items: saveItems, label: "Save as", placement: "top-end" })] })] }), _jsxs("aside", { "aria-label": "Summary settings", className: "flex w-[340px] shrink-0 flex-col", children: [_jsx("div", { className: "min-h-0 flex-1 overflow-y-auto", children: _jsx(SummaryFilters, { options: options, onChange: update, lists: listArray, tags: tagArray }) }), _jsx("footer", { className: "border-t border-line p-4", children: _jsxs("button", { type: "button", onClick: () => generate(true), className: "flex h-9 w-full items-center justify-center gap-2 rounded-row bg-primary text-white", children: [_jsx(Sparkles, { size: 16, "aria-hidden": true }), " Generate"] }) })] })] }));
}

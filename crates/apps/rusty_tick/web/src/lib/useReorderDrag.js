import { useState } from 'react';
/**
 * HTML5 drag-and-drop for a reorderable list. `bind(id)` returns the props for
 * one item; `indicator` says where a drop would land, for drawing a line.
 * Only drags that started from a list using the same `mime` are accepted, so
 * a task cannot be dropped onto the list of tags.
 */
export function useReorderDrag(mime, onDrop) {
    const [dragging, setDragging] = useState(null);
    const [indicator, setIndicator] = useState(null);
    const accepts = (e) => e.dataTransfer.types.includes(mime);
    const bind = (id, enabled = true) => {
        if (!enabled)
            return {};
        return {
            draggable: true,
            onDragStart: (e) => {
                e.dataTransfer.setData(mime, id);
                e.dataTransfer.effectAllowed = 'move';
                setDragging(id);
            },
            onDragEnd: () => {
                setDragging(null);
                setIndicator(null);
            },
            onDragOver: (e) => {
                if (!accepts(e))
                    return;
                e.preventDefault();
                e.dataTransfer.dropEffect = 'move';
                const rect = e.currentTarget.getBoundingClientRect();
                const after = e.clientY > rect.top + rect.height / 2;
                setIndicator((cur) => (cur?.id === id && cur.after === after ? cur : { id, after }));
            },
            onDragLeave: (e) => {
                if (!e.currentTarget.contains(e.relatedTarget))
                    setIndicator((cur) => (cur?.id === id ? null : cur));
            },
            onDrop: (e) => {
                if (!accepts(e))
                    return;
                e.preventDefault();
                const movedId = e.dataTransfer.getData(mime);
                const rect = e.currentTarget.getBoundingClientRect();
                const after = e.clientY > rect.top + rect.height / 2;
                setDragging(null);
                setIndicator(null);
                if (movedId)
                    onDrop(movedId, id, after);
            },
        };
    };
    return { bind, dragging, indicator };
}

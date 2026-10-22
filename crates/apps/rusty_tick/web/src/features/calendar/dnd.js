export const drag = { current: null };
/** Firefox will not start a drag without data on the transfer. */
export function beginDrag(e, info) {
    drag.current = info;
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', info.task.id);
}
export const endDrag = () => {
    drag.current = null;
};
/** A stand-in anchor for a popover that hangs off a rectangle, such as a time slot, not an element. */
export function rectAnchor(rect) {
    const el = document.createElement('div');
    el.getBoundingClientRect = () => rect;
    return el;
}

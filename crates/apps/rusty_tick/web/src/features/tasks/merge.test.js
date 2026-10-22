import { describe, expect, it } from 'vitest';
import { mergeTargets } from './merge';
const task = (id, title, deletedMs = null, parentId = null) => ({ id, title, deletedMs, parentId });
describe('mergeTargets', () => {
    const tasks = [task('a', 'Standup'), task('b', 'standup '), task('c', 'Standup notes'), task('d', 'Standup', 1), task('e', 'Lunch')];
    it('offers the live tasks with the same title, not the duplicate itself', () => {
        expect(mergeTargets(tasks, tasks[0], '').map((t) => t.id)).toEqual(['b']);
    });
    it('searches every live task by title, same title first, up to the limit', () => {
        expect(mergeTargets(tasks, tasks[0], 'STAND').map((t) => t.id)).toEqual(['b', 'c']);
        expect(mergeTargets(tasks, tasks[4], 'stand').map((t) => t.id)).toEqual(['a', 'b', 'c']);
        expect(mergeTargets(tasks, tasks[4], 'stand', 2)).toHaveLength(2);
    });
    it("never offers the duplicate's own subtasks, which go to the Trash with it", () => {
        const parent = task('p', 'Standup');
        expect(mergeTargets([parent, task('c', 'Standup', null, 'p'), task('o', 'Standup')], parent, '').map((t) => t.id)).toEqual(['o']);
    });
});

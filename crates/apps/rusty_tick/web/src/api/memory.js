/**
 * An in-browser `ApiClient` that follows the same rules as `rusty_tick`'s
 * service layer (`service.rs`): the undeletable Inbox, trash and restore,
 * client-chosen ids, `If-Match` versions, tag entities, subtasks moving with
 * their parent. `apiContract.ts` holds the behaviour both must satisfy.
 *
 * State lives in memory and, if a `Storage` is given, is saved to it after
 * every write so a reload keeps it.
 */
import { newId } from '@/lib/id';
import { ConflictError, InvalidError, NotFoundError, StaleError } from './errors';
export const INBOX_ID = '00000000-0000-7000-8000-000000000001';
export const STORAGE_KEY = 'tick-local:memory:v1';
const STEP = 1024;
const DOC_KINDS = ['habit', 'habit_checkin', 'focus', 'prefs', 'summary_template', 'comment', 'filter', 'countdown', 'estimate', 'assignee', 'subscription', 'import_alias'];
const MAX_DOC_BYTES = 64 * 1024;
const clone = (v) => structuredClone(v);
const fail = (message) => {
    throw new InvalidError(422, message);
};
export class MemoryAdapter {
    state;
    now;
    storage;
    constructor(options = {}) {
        this.now = options.now ?? Date.now;
        this.storage = options.storage ?? null;
        this.state = this.load() ?? this.fresh(options.seed);
        this.ensureInbox();
        this.sweepOrphanComments();
        this.save();
    }
    /** Drop comments whose task is gone for good (a trashed one can still come back), left by versions that did not clean up on purge. */
    sweepOrphanComments() {
        if (!Array.isArray(this.state.tasks) || !Array.isArray(this.state.docs))
            return; // a save from an older version may lack either
        const live = new Set(this.state.tasks.map((t) => t.id));
        this.state.docs = this.state.docs.filter((d) => d.kind !== 'comment' || live.has(d.body?.taskId ?? ''));
    }
    /** A store holding a copy of `snapshot`, for replaying pending operations over server truth. */
    static fromSnapshot(snapshot, options = {}) {
        const adapter = new MemoryAdapter(options);
        adapter.state = { lists: clone(snapshot.lists), tasks: clone(snapshot.tasks), tags: clone(snapshot.tags), docs: [] };
        adapter.ensureInbox();
        return adapter;
    }
    /**
     * The current state without a promise. Every mutating method changes state
     * before its first `await` (there are none), so after calling one the result
     * is already visible here.
     */
    peek() {
        return clone({
            inboxId: INBOX_ID,
            serverTimeMs: this.now(),
            lists: [...this.state.lists].sort((a, b) => a.sortOrder - b.sortOrder || cmp(a.id, b.id)),
            tasks: this.state.tasks,
            tags: [...this.state.tags].sort((a, b) => a.sortOrder - b.sortOrder || cmp(a.name, b.name)),
        });
    }
    // ---- lifecycle -----------------------------------------------------
    fresh(seed) {
        const base = seed ? seed(this.now()) : { lists: [], tasks: [], tags: [] };
        return { ...base, docs: [] };
    }
    load() {
        try {
            const raw = this.storage?.getItem(STORAGE_KEY);
            return raw ? JSON.parse(raw) : null;
        }
        catch {
            return null; // an unreadable save is treated as no save
        }
    }
    save() {
        try {
            this.storage?.setItem(STORAGE_KEY, JSON.stringify(this.state));
        }
        catch {
            // Storage full or unavailable: the in-memory state is still correct.
        }
    }
    ensureInbox() {
        if (this.state.lists.some((l) => l.id === INBOX_ID))
            return;
        this.state.lists.unshift(this.makeList(INBOX_ID, 'Inbox', Number.MIN_SAFE_INTEGER / 2));
    }
    makeList(id, name, sortOrder) {
        return { id, name, color: null, archived: false, viewMode: 'list', sortType: '', sortOrder, updatedMs: this.now(), etag: '1' };
    }
    bump(record) {
        record.etag = String(Number(record.etag) + 1);
    }
    checkEtag(current, etag) {
        if (etag !== undefined && etag !== current.etag)
            throw new StaleError(clone(current));
    }
    // ---- snapshot ------------------------------------------------------
    async snapshot() {
        return this.peek();
    }
    // ---- lists ---------------------------------------------------------
    list(id) {
        return this.state.lists.find((l) => l.id === id) ?? fail404('list');
    }
    async createList(input) {
        const id = input.id ?? newId(this.now());
        if (this.state.lists.some((l) => l.id === id))
            throw new ConflictError(`${id} already exists`);
        const last = Math.max(0, ...this.state.lists.filter((l) => l.id !== INBOX_ID).map((l) => l.sortOrder));
        const list = this.makeList(id, cleanName(input.name), this.state.lists.length > 1 ? last + STEP : 0);
        list.color = cleanColor(input.color ?? null);
        this.state.lists.push(list);
        this.save();
        return clone(list);
    }
    async updateList(id, patch, etag) {
        const list = this.list(id);
        this.checkEtag(list, etag);
        if (id === INBOX_ID && patch.archived === true)
            fail('the Inbox cannot be archived');
        if (patch.name !== undefined)
            list.name = cleanName(patch.name);
        if (patch.color !== undefined)
            list.color = cleanColor(patch.color);
        if (patch.archived !== undefined)
            list.archived = patch.archived;
        if (patch.viewMode !== undefined) {
            if (!['list', 'kanban', 'timeline', 'matrix'].includes(patch.viewMode))
                throw new InvalidError(400, 'unknown viewMode');
            list.viewMode = patch.viewMode;
        }
        if (patch.sortType !== undefined)
            list.sortType = patch.sortType;
        if (patch.sortOrder !== undefined)
            list.sortOrder = patch.sortOrder;
        list.updatedMs = this.now();
        this.bump(list);
        this.save();
        return clone(list);
    }
    async deleteList(id) {
        if (id === INBOX_ID)
            fail('the Inbox cannot be deleted');
        this.list(id);
        const now = this.now();
        for (const t of this.state.tasks.filter((t) => t.listId === id && t.deletedMs === null)) {
            t.deletedMs = now;
            this.touch(t);
        }
        this.state.lists = this.state.lists.filter((l) => l.id !== id);
        this.save();
    }
    // ---- tasks ---------------------------------------------------------
    task(id) {
        return this.state.tasks.find((t) => t.id === id) ?? fail404('task');
    }
    children(parent) {
        return this.state.tasks.filter((t) => t.parentId === parent.id);
    }
    touch(t) {
        t.updatedMs = this.now();
        this.bump(t);
    }
    async createTask(input) {
        this.list(input.listId);
        if (input.parentId) {
            const parent = this.task(input.parentId);
            if (parent.listId !== input.listId)
                fail("a subtask must be in its parent's list");
            if (parent.parentId)
                fail('subtasks nest one level deep');
        }
        const id = input.id ?? newId(this.now());
        if (this.state.tasks.some((t) => t.id === id))
            throw new ConflictError(`${id} already exists`);
        const now = this.now();
        const inList = this.state.tasks.filter((t) => t.listId === input.listId);
        const task = {
            id,
            listId: input.listId,
            parentId: input.parentId ?? null,
            title: cleanTitle(input.title),
            notes: cleanNotes(input.notes ?? ''),
            kind: input.kind ?? 'text',
            status: 'open',
            priority: cleanPriority(input.priority ?? 0),
            startMs: input.startMs ?? null,
            dueMs: input.dueMs ?? null,
            isAllDay: input.isAllDay ?? false,
            timeZone: input.timeZone ?? '',
            reminders: input.reminders ?? [],
            repeatFlag: input.repeatFlag ?? '',
            exDates: [],
            items: cleanItems(input.items ?? []),
            tags: this.cleanTags(input.tags ?? []),
            sortOrder: input.sortOrder ?? (inList.length ? Math.max(...inList.map((t) => t.sortOrder)) + STEP : 0),
            createdMs: now,
            updatedMs: now,
            completedMs: null,
            deletedMs: null,
            etag: '1',
        };
        this.state.tasks.push(task);
        this.save();
        return clone(task);
    }
    async updateTask(id, patch, etag) {
        const task = this.task(id);
        this.checkEtag(task, etag);
        if (patch.title !== undefined)
            task.title = cleanTitle(patch.title);
        if (patch.notes !== undefined)
            task.notes = cleanNotes(patch.notes);
        if (patch.kind !== undefined)
            task.kind = patch.kind;
        if (patch.status !== undefined && patch.status !== task.status) {
            task.status = patch.status;
            task.completedMs = patch.status === 'open' ? null : (task.completedMs ?? this.now());
        }
        if (patch.priority !== undefined)
            task.priority = cleanPriority(patch.priority);
        if (patch.startMs !== undefined)
            task.startMs = patch.startMs;
        if (patch.dueMs !== undefined)
            task.dueMs = patch.dueMs;
        if (patch.isAllDay !== undefined)
            task.isAllDay = patch.isAllDay;
        if (patch.timeZone !== undefined)
            task.timeZone = patch.timeZone;
        if (patch.reminders !== undefined)
            task.reminders = patch.reminders;
        if (patch.repeatFlag !== undefined)
            task.repeatFlag = patch.repeatFlag;
        if (patch.exDates !== undefined)
            task.exDates = cleanExDates(patch.exDates);
        if (patch.items !== undefined)
            task.items = cleanItems(patch.items);
        if (patch.tags !== undefined)
            task.tags = this.cleanTags(patch.tags);
        if (patch.sortOrder !== undefined)
            task.sortOrder = patch.sortOrder;
        if (patch.listId !== undefined && patch.listId !== task.listId) {
            this.list(patch.listId);
            if (task.parentId)
                fail('a subtask moves with its parent');
            for (const child of this.children(task)) {
                child.listId = patch.listId;
                this.touch(child);
            }
            task.listId = patch.listId;
        }
        this.touch(task);
        this.save();
        return clone(task);
    }
    async trashTask(id, etag) {
        const task = this.task(id);
        this.checkEtag(task, etag);
        const now = this.now();
        for (const child of this.children(task)) {
            child.deletedMs ??= now;
            this.touch(child);
        }
        task.deletedMs ??= now;
        this.touch(task);
        this.save();
    }
    async restoreTask(id) {
        const task = this.task(id);
        const trashedAt = task.deletedMs;
        if (!this.state.lists.some((l) => l.id === task.listId)) {
            for (const child of this.children(task))
                child.listId = INBOX_ID;
            task.listId = INBOX_ID;
        }
        for (const child of this.children(task)) {
            if (child.deletedMs === trashedAt) {
                child.deletedMs = null;
                this.touch(child);
            }
        }
        task.deletedMs = null;
        this.touch(task);
        this.save();
        return clone(task);
    }
    async purgeTask(id) {
        const task = this.task(id);
        const gone = new Set([task.id, ...this.children(task).map((c) => c.id)]);
        this.state.tasks = this.state.tasks.filter((t) => !gone.has(t.id));
        this.dropComments(gone);
        this.save();
    }
    /** A task's comments go with it for good (but not to the trash, so a restore loses nothing). */
    dropComments(taskIds) {
        this.state.docs = this.state.docs.filter((d) => !(d.kind === 'comment' && taskIds.has(d.body?.taskId ?? '')));
    }
    async emptyTrash() {
        const before = this.state.tasks.length;
        this.dropComments(new Set(this.state.tasks.filter((t) => t.deletedMs !== null).map((t) => t.id)));
        this.state.tasks = this.state.tasks.filter((t) => t.deletedMs === null);
        this.save();
        return before - this.state.tasks.length;
    }
    // ---- tags ----------------------------------------------------------
    tag(name) {
        return this.state.tags.find((t) => t.name === tagName(name)) ?? fail404('tag');
    }
    nextTagOrder() {
        return this.state.tags.length ? Math.max(...this.state.tags.map((t) => t.sortOrder)) + STEP : 0;
    }
    /** Lowercase, de-duplicate and bound `input`; create any tag that does not exist. */
    cleanTags(input) {
        const names = [];
        const labels = [];
        for (const raw of input) {
            const label = raw.trim();
            const name = tagName(label);
            if (!name || names.includes(name))
                continue;
            if ([...label].length > 64)
                fail('a tag is longer than 64 characters');
            names.push(name);
            labels.push(label);
        }
        if (names.length > 20)
            fail('more than 20 tags');
        names.forEach((name, i) => {
            if (!this.state.tags.some((t) => t.name === name)) {
                this.state.tags.push({ name, label: labels[i], color: null, parent: null, sortOrder: this.nextTagOrder(), etag: '1' });
            }
        });
        return names;
    }
    async createTag(label, color = null) {
        const clean = label.trim();
        if (!clean)
            fail('tag must not be empty');
        if ([...clean].length > 64)
            fail('tag is longer than 64 characters');
        if (this.state.tags.some((t) => t.name === tagName(clean)))
            throw new ConflictError(`tag "${clean}" already exists`);
        const tag = { name: tagName(clean), label: clean, color: cleanColor(color), parent: null, sortOrder: this.nextTagOrder(), etag: '1' };
        this.state.tags.push(tag);
        this.save();
        return clone(tag);
    }
    async updateTag(name, patch, etag) {
        const tag = this.tag(name);
        this.checkEtag(tag, etag);
        if (patch.color !== undefined)
            tag.color = cleanColor(patch.color);
        if (patch.parent !== undefined) {
            if (patch.parent === null)
                tag.parent = null;
            else {
                const parent = tagName(patch.parent);
                if (parent === tag.name || !this.state.tags.some((t) => t.name === parent))
                    fail('parent must be another existing tag');
                tag.parent = parent;
            }
        }
        if (patch.sortOrder !== undefined)
            tag.sortOrder = patch.sortOrder;
        this.bump(tag);
        this.save();
        return clone(tag);
    }
    async renameTag(name, label) {
        const tag = this.tag(name);
        const clean = label.trim();
        if (!clean)
            fail('tag must not be empty');
        const next = tagName(clean);
        if (next !== tag.name && this.state.tags.some((t) => t.name === next))
            throw new ConflictError(`tag "${clean}" already exists`);
        const old = tag.name;
        tag.label = clean;
        tag.name = next;
        this.bump(tag);
        if (next !== old) {
            for (const t of this.state.tags)
                if (t.parent === old)
                    t.parent = next;
            this.retag(old, next);
        }
        this.save();
        return clone(tag);
    }
    async deleteTag(name) {
        const tag = this.tag(name);
        this.state.tags = this.state.tags.filter((t) => t !== tag);
        for (const t of this.state.tags)
            if (t.parent === tag.name)
                t.parent = null;
        this.retag(tag.name, null);
        this.save();
    }
    retag(from, to) {
        for (const task of this.state.tasks.filter((t) => t.tags.includes(from))) {
            const next = task.tags.flatMap((g) => (g === from ? (to ? [to] : []) : [g]));
            task.tags = [...new Set(next)];
            this.touch(task);
        }
    }
    // ---- client documents ----------------------------------------------
    checkKind(kind) {
        if (!DOC_KINDS.includes(kind))
            fail(`unknown document kind "${kind}"`);
    }
    async listDocs(kind) {
        this.checkKind(kind);
        return clone(this.state.docs.filter((d) => d.kind === kind).sort((a, b) => a.updatedMs - b.updatedMs || cmp(a.id, b.id)));
    }
    async putDoc(kind, id, body) {
        this.checkKind(kind);
        if (JSON.stringify(body).length > MAX_DOC_BYTES)
            fail(`document is larger than ${MAX_DOC_BYTES} bytes`);
        const existing = this.state.docs.find((d) => d.id === id);
        if (existing && existing.kind !== kind)
            throw new ConflictError('id belongs to another kind');
        const doc = { id, kind, body: clone(body), updatedMs: this.now() };
        this.state.docs = [...this.state.docs.filter((d) => d.id !== id), doc];
        this.save();
        return clone(doc);
    }
    async fetchIcs() {
        throw new InvalidError(400, 'Calendar subscriptions need the server: the browser demo cannot fetch feeds');
    }
    async deleteDoc(kind, id) {
        this.checkKind(kind);
        const doc = this.state.docs.find((d) => d.id === id && d.kind === kind);
        if (!doc)
            throw new NotFoundError('document not found');
        this.state.docs = this.state.docs.filter((d) => d !== doc);
        this.save();
    }
}
// ---- validation, shared with the rules in service.rs --------------------
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const fail404 = (what) => {
    throw new NotFoundError(`${what} not found`);
};
export const tagName = (label) => label.trim().toLowerCase();
function cleanName(name) {
    const n = name.trim();
    if (!n)
        fail('name must not be empty');
    if ([...n].length > 200)
        fail('name is longer than 200 characters');
    return n;
}
function cleanTitle(title) {
    const t = title.trim();
    if (!t)
        fail('title must not be empty');
    if ([...t].length > 500)
        fail('title is longer than 500 characters');
    return t;
}
function cleanNotes(notes) {
    if ([...notes].length > 100_000)
        fail('notes are longer than 100000 characters');
    return notes;
}
function cleanExDates(exDates) {
    if (exDates.length > 1000)
        fail('more than 1000 exDates');
    return exDates;
}
function cleanColor(color) {
    if (color === null)
        return null;
    if (!/^#[0-9a-f]{6}$/i.test(color))
        fail('color must look like #rrggbb');
    return color.toLowerCase();
}
function cleanPriority(p) {
    if (p !== 0 && p !== 1 && p !== 3 && p !== 5)
        fail('priority must be 0, 1, 3 or 5');
    return p;
}
function cleanItems(items) {
    if (items.length > 200)
        fail('more than 200 checklist items');
    return items.map((i) => ({ ...i, title: cleanTitle(i.title), done: !!i.done, sortOrder: i.sortOrder ?? 0 }));
}

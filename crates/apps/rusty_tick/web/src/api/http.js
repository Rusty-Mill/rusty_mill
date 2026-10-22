import { ApiError, ConflictError, InvalidError, NetworkError, NotFoundError, StaleError, UnauthorizedError } from './errors';
export class HttpAdapter {
    options;
    base;
    fetchImpl;
    timeoutMs;
    constructor(options) {
        this.options = options;
        this.base = `${options.baseUrl ?? ''}/api/v1`;
        this.fetchImpl = options.fetchImpl ?? ((...args) => fetch(...args));
        this.timeoutMs = options.timeoutMs ?? 15_000;
    }
    async request(method, path, body, etag) {
        const headers = { Accept: 'application/json' };
        const token = this.options.getToken();
        if (token)
            headers.Authorization = `Bearer ${token}`;
        if (body !== undefined)
            headers['Content-Type'] = 'application/json';
        if (etag !== undefined)
            headers['If-Match'] = `"${etag}"`;
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), this.timeoutMs);
        let response;
        try {
            response = await this.fetchImpl(this.base + path, {
                method,
                headers,
                body: body === undefined ? undefined : JSON.stringify(body),
                signal: controller.signal,
                cache: 'no-store',
            });
        }
        catch {
            throw new NetworkError(); // refused, offline, DNS, or timed out
        }
        finally {
            clearTimeout(timer);
        }
        if (response.status === 204)
            return undefined;
        const text = await response.text();
        const data = text ? JSON.parse(text) : undefined;
        if (!response.ok)
            throw toError(response.status, data);
        return data;
    }
    snapshot() {
        return this.request('GET', '/snapshot');
    }
    createList(input) {
        return this.request('POST', '/lists', input);
    }
    updateList(id, patch, etag) {
        return this.request('PATCH', `/lists/${id}`, patch, etag);
    }
    deleteList(id) {
        return this.request('DELETE', `/lists/${id}`);
    }
    createTask(input) {
        return this.request('POST', '/tasks', input);
    }
    updateTask(id, patch, etag) {
        return this.request('PATCH', `/tasks/${id}`, patch, etag);
    }
    trashTask(id, etag) {
        return this.request('DELETE', `/tasks/${id}`, undefined, etag);
    }
    restoreTask(id) {
        return this.request('POST', `/tasks/${id}/restore`);
    }
    purgeTask(id) {
        return this.request('DELETE', `/tasks/${id}?permanent=true`);
    }
    async emptyTrash() {
        const r = await this.request('DELETE', '/trash');
        return r.purged;
    }
    createTag(label, color = null) {
        return this.request('POST', '/tags', { label, color });
    }
    updateTag(name, patch, etag) {
        return this.request('PATCH', `/tags/${encodeURIComponent(name)}`, patch, etag);
    }
    renameTag(name, label) {
        return this.request('POST', `/tags/${encodeURIComponent(name)}/rename`, { label });
    }
    deleteTag(name) {
        return this.request('DELETE', `/tags/${encodeURIComponent(name)}`);
    }
    async listDocs(kind) {
        const r = await this.request('GET', `/docs/${kind}`);
        return r.docs;
    }
    putDoc(kind, id, body) {
        return this.request('PUT', `/docs/${kind}/${id}`, body);
    }
    deleteDoc(kind, id) {
        return this.request('DELETE', `/docs/${kind}/${id}`);
    }
    async fetchIcs(url) {
        return (await this.request('POST', '/fetch-ics', { url })).text;
    }
}
function toError(status, body) {
    const message = body?.error?.message ?? `request failed (${status})`;
    switch (status) {
        case 401:
            return new UnauthorizedError(message);
        case 404:
            return new NotFoundError(message);
        case 409:
            return new ConflictError(message);
        case 412:
            return new StaleError(body?.current);
        case 400:
        case 422:
            return new InvalidError(status, message);
        default:
            return new ApiError(status, body?.error?.code ?? 'internal', message);
    }
}

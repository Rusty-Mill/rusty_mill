/** A `Storage` view that namespaces every key, so demo and server mode never share a queue or cache. */
export function prefixStorage(inner: Storage, prefix: string): Storage {
  return {
    get length() {
      return inner.length
    },
    clear: () => {
      for (const k of Object.keys(inner)) if (k.startsWith(prefix)) inner.removeItem(k)
    },
    getItem: (k) => inner.getItem(prefix + k),
    key: (i) => inner.key(i),
    removeItem: (k) => inner.removeItem(prefix + k),
    setItem: (k, v) => inner.setItem(prefix + k, v),
  }
}

// RFC 6901 JSON Pointer, RFC 6902 JSON Patch (atomic apply) and RFC 7386
// Merge Patch. The mirror of the rusty_json_patch crate, so STATE_DELTA
// and ACTIVITY_DELTA reduce identically on both sides.

import type { Json } from "./types.js";

export class PatchError extends Error {
  override name = "PatchError";
}

type Container = Json[] | { [key: string]: Json };

function isObject(value: Json): value is { [key: string]: Json } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseIndex(token: string): number | undefined {
  if (token === "" || (token.length > 1 && token.startsWith("0"))) return undefined;
  if (!/^[0-9]+$/.test(token)) return undefined;
  return Number(token);
}

/** Parses a pointer into unescaped tokens; the empty pointer is `[]`. */
export function parsePointer(pointer: string): string[] {
  if (pointer === "") return [];
  if (!pointer.startsWith("/")) throw new PatchError(`invalid JSON pointer: ${JSON.stringify(pointer)}`);
  return pointer
    .slice(1)
    .split("/")
    .map((token) => {
      if (/~(?![01])/.test(token)) throw new PatchError(`invalid JSON pointer: ${JSON.stringify(pointer)}`);
      return token.replace(/~1/g, "/").replace(/~0/g, "~");
    });
}

function step(current: Json, token: string): Json | undefined {
  if (Array.isArray(current)) {
    const index = parseIndex(token);
    return index === undefined ? undefined : current[index];
  }
  if (isObject(current)) return Object.prototype.hasOwnProperty.call(current, token) ? current[token] : undefined;
  return undefined;
}

/** The value a pointer names, or `undefined`. */
export function resolvePointer(doc: Json, pointer: string): Json | undefined {
  let current: Json | undefined = doc;
  for (const token of parsePointer(pointer)) {
    if (current === undefined) return undefined;
    current = step(current, token);
  }
  return current;
}

interface Slot {
  parent: Container;
  key: string | number;
}

function slot(doc: Json, pointer: string, allowAppend: boolean): Slot {
  const tokens = parsePointer(pointer);
  if (tokens.length === 0) throw new PatchError(`path not found: ${pointer}`);
  const last = tokens[tokens.length - 1]!;
  let parent: Json | undefined = doc;
  for (const token of tokens.slice(0, -1)) {
    parent = parent === undefined ? undefined : step(parent, token);
  }
  if (parent === undefined) throw new PatchError(`path not found: ${pointer}`);
  if (Array.isArray(parent)) {
    if (last === "-" && allowAppend) return { parent, key: parent.length };
    const index = parseIndex(last);
    const limit = allowAppend ? parent.length : parent.length - 1;
    if (index === undefined || index > limit || index < 0) throw new PatchError(`invalid array index at ${pointer}`);
    return { parent, key: index };
  }
  if (isObject(parent)) return { parent, key: last };
  throw new PatchError(`path not found: ${pointer}`);
}

function add(doc: Json, pointer: string, value: Json): Json {
  if (pointer === "") return value;
  const { parent, key } = slot(doc, pointer, true);
  if (Array.isArray(parent)) parent.splice(key as number, 0, value);
  else parent[key as string] = value;
  return doc;
}

function remove(doc: Json, pointer: string): [Json, Json] {
  if (pointer === "") return [null, doc];
  const { parent, key } = slot(doc, pointer, false);
  if (Array.isArray(parent)) return [doc, parent.splice(key as number, 1)[0]!];
  if (!Object.prototype.hasOwnProperty.call(parent, key)) throw new PatchError(`path not found: ${pointer}`);
  const removed = parent[key as string]!;
  delete parent[key as string];
  return [doc, removed];
}

function deepEqual(a: Json, b: Json): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => deepEqual(v, b[i]!));
  if (isObject(a) && isObject(b)) {
    const keys = Object.keys(a);
    return keys.length === Object.keys(b).length && keys.every((k) => k in b && deepEqual(a[k]!, b[k]!));
  }
  return false;
}

function isPrefix(from: string, path: string): boolean {
  const a = parsePointer(from);
  const b = parsePointer(path);
  return a.length < b.length && a.every((t, i) => t === b[i]);
}

function op(obj: Json, name: string): string {
  if (!isObject(obj)) throw new PatchError("invalid JSON patch: operation is not an object");
  const value = obj[name];
  if (typeof value !== "string") throw new PatchError(`invalid JSON patch: operation lacks ${JSON.stringify(name)}`);
  return value;
}

function valueOf(obj: { [key: string]: Json }): Json {
  if (!("value" in obj)) throw new PatchError('invalid JSON patch: operation lacks "value"');
  return obj["value"]!;
}

/**
 * Applies an RFC 6902 patch and returns the new document. Atomic: on any
 * error the input is untouched (the patch runs on a deep copy).
 */
export function applyPatch(doc: Json, patch: Json): Json {
  if (!Array.isArray(patch)) throw new PatchError("invalid JSON patch: patch is not an array");
  let work: Json = structuredClone(doc);
  for (const entry of patch) {
    const kind = op(entry, "op");
    const path = op(entry, "path");
    const obj = entry as { [key: string]: Json };
    switch (kind) {
      case "add":
        work = add(work, path, structuredClone(valueOf(obj)));
        break;
      case "remove":
        [work] = remove(work, path);
        break;
      case "replace": {
        if (resolvePointer(work, path) === undefined) throw new PatchError(`path not found: ${path}`);
        work = add(path === "" ? work : remove(work, path)[0], path, structuredClone(valueOf(obj)));
        break;
      }
      case "move": {
        const from = op(entry, "from");
        if (isPrefix(from, path)) throw new PatchError(`cannot move a value into itself: ${path}`);
        if (from === path) break;
        let moved: Json;
        [work, moved] = remove(work, from);
        work = add(work, path, moved);
        break;
      }
      case "copy": {
        const from = op(entry, "from");
        const found = resolvePointer(work, from);
        if (found === undefined) throw new PatchError(`path not found: ${from}`);
        work = add(work, path, structuredClone(found));
        break;
      }
      case "test": {
        const found = resolvePointer(work, path);
        if (found === undefined) throw new PatchError(`path not found: ${path}`);
        if (!deepEqual(found, valueOf(obj))) throw new PatchError(`test failed at ${path}`);
        break;
      }
      default:
        throw new PatchError(`invalid JSON patch: unknown op ${JSON.stringify(kind)}`);
    }
  }
  return work;
}

/** RFC 7386: returns the merged document. Objects merge; `null` deletes. */
export function mergePatch(target: Json, patch: Json): Json {
  if (!isObject(patch)) return structuredClone(patch);
  const result: { [key: string]: Json } = isObject(target) ? { ...target } : {};
  for (const [key, change] of Object.entries(patch)) {
    if (change === null) delete result[key];
    else result[key] = mergePatch(result[key] ?? null, change);
  }
  return result;
}

export interface ImagePreviewSnapshot { url: string | null; failed: boolean }
export interface ImagePreviewStore {
  snapshot: (key: string) => ImagePreviewSnapshot | null;
  subscribe: (listener: () => void) => () => void;
  retain: (key: string) => () => void;
  fail: (key: string) => void;
}
interface Entry {
  key: string; refs: number; running: boolean; bytes: number;
  snapshot: ImagePreviewSnapshot;
}

/** Workspace-scoped presentation cache, not an authorization or disk cache.
 * The owner must invalidate on grant/version changes and clear on workspace exit.
 * Only the injected loader obtains new bytes; mounted cards share its result. */
export function createImagePreviewStore(load: (key: string) => Promise<Blob>, options: {
  concurrency?: number; maxBytes?: number; maxEntries?: number;
  createUrl?: (blob: Blob) => string; revokeUrl?: (url: string) => void;
} = {}) {
  const entries = new Map<string, Entry>(), listeners = new Set<() => void>();
  const concurrency = options.concurrency ?? 4, maxBytes = options.maxBytes ?? 64 * 1024 * 1024;
  const maxEntries = options.maxEntries ?? 512;
  const createUrl = options.createUrl ?? (blob => URL.createObjectURL(blob));
  const revokeUrl = options.revokeUrl ?? (url => URL.revokeObjectURL(url));
  let active = 0, scheduled = false;
  const notify = () => { for (const listener of listeners) listener(); };
  const remove = (entry: Entry) => {
    if (entries.get(entry.key) !== entry) return;
    entries.delete(entry.key);
    if (entry.snapshot.url) revokeUrl(entry.snapshot.url);
  };
  const trim = () => {
    let total = [...entries.values()].reduce((sum, entry) => sum + entry.bytes, 0);
    for (const entry of entries.values()) {
      if (total <= maxBytes && entries.size <= maxEntries) break;
      if (!entry.refs && !entry.running) { total -= entry.bytes; remove(entry); }
    }
  };
  const pump = () => {
    scheduled = false;
    for (const entry of entries.values()) {
      if (active >= concurrency) break;
      if (!entry.refs || entry.running || entry.snapshot.url || entry.snapshot.failed) continue;
      entry.running = true; active++;
      void Promise.resolve().then(() => load(entry.key)).then(blob => {
        if (entries.get(entry.key) !== entry) return;
        if (!blob.size || blob.size > maxBytes) throw new Error('preview_size_limit');
        entry.bytes = blob.size;
        trim();
        if ([...entries.values()].reduce((sum, item) => sum + item.bytes, 0) > maxBytes) throw new Error('preview_cache_full');
        entry.snapshot = { url: createUrl(blob), failed: false };
      }).catch(() => {
        if (entries.get(entry.key) === entry) { entry.bytes = 0; entry.snapshot = { url: null, failed: true }; }
      }).finally(() => {
        active--; entry.running = false;
        if (entries.get(entry.key) === entry && !entry.refs && entry.snapshot.failed) remove(entry);
        trim(); notify(); schedule();
      });
    }
  };
  const schedule = () => { if (!scheduled) { scheduled = true; queueMicrotask(pump); } };
  return {
    snapshot: (key: string) => entries.get(key)?.snapshot ?? null,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    retain(key: string) {
      let entry = entries.get(key);
      if (!entry) {
        entry = { key, refs: 0, running: false, bytes: 0, snapshot: { url: null, failed: false } };
        entries.set(key, entry);
      }
      entry.refs++;
      // Touch the LRU without changing the immutable snapshot.
      entries.delete(key); entries.set(key, entry);
      schedule();
      let released = false;
      return () => {
        if (released) return; released = true;
        entry.refs--;
        if (!entry.refs && !entry.running && !entry.snapshot.url) remove(entry);
        trim(); schedule();
      };
    },
    fail(key: string) {
      const entry = entries.get(key);
      if (!entry) return;
      if (entry.snapshot.url) revokeUrl(entry.snapshot.url);
      entry.bytes = 0; entry.snapshot = { url: null, failed: true }; notify();
    },
    invalidate(keep: (key: string) => boolean = () => false) {
      for (const entry of entries.values()) if (!keep(entry.key)) remove(entry);
      notify(); schedule();
    },
  };
}

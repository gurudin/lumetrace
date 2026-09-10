import test from 'node:test';
import assert from 'node:assert/strict';
import { createImagePreviewStore } from '../src/shared/extensions/imagePreviewStore.ts';

const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function harness(options: { maxEntries?: number; maxBytes?: number } = {}) {
  const calls: string[] = [], revoked: string[] = [];
  const pending = new Map<string, { resolve: (value: Blob) => void; reject: (error: Error) => void }>();
  let next = 0;
  const store = createImagePreviewStore(key => {
    calls.push(key);
    return new Promise<Blob>((resolve, reject) => pending.set(key, { resolve, reject }));
  }, { ...options, createUrl: () => `blob:synthetic-${++next}`, revokeUrl: url => { revoked.push(url); } });
  const finish = async (key: string, size = 4) => { pending.get(key)!.resolve(new Blob(['x'.repeat(size)])); await flush(); };
  return { store, calls, revoked, pending, finish };
}

test('returning to a loaded image is synchronous while four later reads remain stalled', async () => {
  const { store, calls, finish } = harness();
  const release = store.retain('first:v1'); await flush(); await finish('first:v1');
  const first = store.snapshot('first:v1'); release();
  for (let i = 0; i < 4; i++) store.retain(`slow:${i}`);
  await flush();
  store.retain('first:v1');
  assert.equal(store.snapshot('first:v1'), first, 'no loading snapshot even before the next microtask');
  assert.ok(first?.url); assert.equal(first.failed, false);
  await flush(); assert.equal(calls.filter(key => key === 'first:v1').length, 1);
});

test('offscreen queued reads are discarded and duplicate mounted cards share one request', async () => {
  const { store, calls, finish } = harness();
  for (let i = 0; i < 4; i++) store.retain(`active:${i}`);
  await flush();
  const releases = Array.from({ length: 100 }, (_, i) => store.retain(`offscreen:${i}`));
  releases.forEach(release => release());
  store.retain('visible'); store.retain('visible');
  await finish('active:0');
  assert.deepEqual(calls, ['active:0', 'active:1', 'active:2', 'active:3', 'visible']);
});

test('revocation clears ready URLs and rejects late in-flight results; no cross-store reuse', async () => {
  const { store, revoked, finish } = harness();
  store.retain('ready'); store.retain('late'); await flush(); await finish('ready');
  const url = store.snapshot('ready')!.url;
  store.invalidate(); await finish('late');
  assert.equal(store.snapshot('ready'), null); assert.equal(store.snapshot('late'), null);
  assert.deepEqual(revoked, [url]);
  assert.equal(harness().store.snapshot('ready'), null);
});

test('version invalidation keeps unchanged previews and cache eviction is bounded', async () => {
  const { store, revoked, finish } = harness({ maxEntries: 2, maxBytes: 8 });
  for (const key of ['a:v1', 'b:v1', 'c:v1']) {
    const release = store.retain(key); await flush(); await finish(key); release();
  }
  assert.equal(store.snapshot('a:v1'), null); assert.equal(revoked.length, 1);
  const unchanged = store.snapshot('c:v1');
  store.invalidate(key => key === 'c:v1');
  assert.equal(store.snapshot('b:v1'), null); assert.equal(store.snapshot('c:v1'), unchanged);
  assert.equal(store.snapshot('c:v2'), null);
});

test('failed and undecodable images settle without unlimited retry and retry after remount', async () => {
  const { store, calls, pending, finish, revoked } = harness();
  const release = store.retain('bad'); await flush();
  pending.get('bad')!.reject(new Error('denied')); await flush();
  assert.equal(store.snapshot('bad')?.failed, true); assert.equal(calls.length, 1);
  release(); const releaseAgain = store.retain('bad'); await flush(); await finish('bad');
  const url = store.snapshot('bad')!.url; store.fail('bad');
  assert.equal(store.snapshot('bad')?.failed, true); assert.deepEqual(revoked, [url]);
  releaseAgain(); releaseAgain(); assert.equal(store.snapshot('bad'), null);
});

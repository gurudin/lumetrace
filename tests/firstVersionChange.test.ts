import assert from "node:assert/strict";
import test from "node:test";
import { createFirstVersionChangeClaim, firstVersionChangeStorageKey, savedVersionNotification } from "../src/pages/file-space/firstVersionChange.ts";

const newVersion = { id: "v2", versionNumber: 2, isCurrent: true, origin: "user_edit", producedAt: 100 };
const notification = savedVersionNotification("file-1", "notes.md", 1, [newVersion])!;

test("only a newly saved, comparable user version qualifies for guidance", () => {
  assert.equal(savedVersionNotification("file-1", "notes.md", 1, []), null);
  assert.equal(savedVersionNotification("file-1", "notes.md", 1, [{ ...newVersion, versionNumber: 1 }]), null);
  assert.equal(savedVersionNotification("file-1", "notes.md", 2, [newVersion]), null);
  assert.equal(savedVersionNotification("file-1", "notes.md", 1, [{ ...newVersion, origin: "task" }]), null);
  assert.equal(savedVersionNotification("file-1", "notes.md", 1, [{ ...newVersion, isCurrent: false }]), null);
  assert.deepEqual(notification, { fileId: "file-1", fileName: "notes.md", versionId: "v2", versionNumber: 2, createdAt: 100 });
});

test("initial import does not consume guidance; edits share one claim across files and restarts", () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
  const claim = createFirstVersionChangeClaim(() => storage);
  assert.equal(claim({ ...notification, versionNumber: 1 }), false);
  assert.equal(values.has(firstVersionChangeStorageKey), false);
  assert.equal(claim(notification), true);
  assert.equal(claim(notification), false);
  assert.equal(claim({ ...notification, fileId: "file-2", versionId: "other-v2" }), false);
  assert.equal(createFirstVersionChangeClaim(() => storage)(notification), false);
});

test("unavailable storage does not break saving or repeat guidance within the session", () => {
  const claim = createFirstVersionChangeClaim(() => { throw new Error("Storage disabled"); });
  assert.equal(claim(notification), true);
  assert.equal(claim(notification), false);
});

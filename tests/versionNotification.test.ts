import assert from "node:assert/strict";
import test from "node:test";
import {
  mergeVersionNotifications,
  type FileSpaceVersionNotification,
} from "../src/pages/file-space/versionNotification.ts";

function notification(versionId: string, fileId = "file-1"): FileSpaceVersionNotification {
  return {
    versionId,
    fileId,
    fileName: `${fileId}.md`,
    versionNumber: Number(versionId.replace(/\D/g, "")) || 1,
    createdAt: 1,
  };
}

test("automatic version notifications remain ordered and deduplicate native signals", () => {
  const first = notification("version-1");
  const second = notification("version-2", "file-2");
  assert.deepEqual(
    mergeVersionNotifications([first], [first, second]),
    [first, second],
  );
});

test("automatic version notification queue remains bounded", () => {
  const notifications = Array.from({ length: 105 }, (_, index) => (
    notification(`version-${index + 1}`)
  ));
  const merged = mergeVersionNotifications([], notifications);
  assert.equal(merged.length, 100);
  assert.equal(merged[0]?.versionId, "version-6");
  assert.equal(merged.at(-1)?.versionId, "version-105");
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  backgroundStatusCount,
  shouldShowBackgroundStatus,
  type BackgroundStatus,
} from "../src/pages/file-space/backgroundStatusPresentation.ts";

function backgroundStatus(state: BackgroundStatus["state"]): BackgroundStatus {
  return {
    state,
    paused: state === "paused",
    contentIndex: {
      state: state === "running" ? "running" : "ready",
      completedFiles: 8,
      totalFiles: 12,
      pendingFiles: 3,
      failedFiles: state === "attention" ? 2 : 0,
    },
    semanticIndex: {
      state: state === "running" ? "running" : "ready",
      completedFiles: 10,
      totalFiles: 12,
      pendingFiles: 2,
      failedFiles: state === "attention" ? 1 : 0,
    },
    semanticModelInstalled: true,
    watcher: {
      state: state === "attention" ? "failed" : "watching",
    },
    updatedAt: 1,
  };
}

test("hides the toolbar background status when all work is ready", () => {
  assert.equal(shouldShowBackgroundStatus(backgroundStatus("ready")), false);
  assert.equal(shouldShowBackgroundStatus(backgroundStatus("running")), true);
  assert.equal(shouldShowBackgroundStatus(backgroundStatus("paused")), true);
  assert.equal(shouldShowBackgroundStatus(backgroundStatus("attention")), true);
});

test("shows pending work while running and failures when attention is required", () => {
  assert.equal(backgroundStatusCount(backgroundStatus("running")), 5);
  assert.equal(backgroundStatusCount(backgroundStatus("attention")), 4);
  assert.equal(backgroundStatusCount(backgroundStatus("paused")), 0);
});

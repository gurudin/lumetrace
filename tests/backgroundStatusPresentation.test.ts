import assert from "node:assert/strict";
import test from "node:test";
import {
  backgroundStatusCount,
  backgroundStatusIndicatorState,
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

test("keeps the toolbar entry available when the status request fails", () => {
  assert.equal(backgroundStatusIndicatorState(null, true), "unavailable");
  assert.equal(shouldShowBackgroundStatus(null, true), true);

  const previouslyReady = backgroundStatus("ready");
  assert.equal(backgroundStatusIndicatorState(previouslyReady, true), "unavailable");
  assert.equal(shouldShowBackgroundStatus(previouslyReady, true), true);
});

test("restores the real toolbar presentation after status requests recover", () => {
  assert.equal(backgroundStatusIndicatorState(backgroundStatus("running"), false), "running");
  assert.equal(backgroundStatusIndicatorState(backgroundStatus("attention"), false), "attention");
  assert.equal(backgroundStatusIndicatorState(backgroundStatus("ready"), false), null);
});

test("shows pending work while running and failures when attention is required", () => {
  assert.equal(backgroundStatusCount(backgroundStatus("running")), 5);
  assert.equal(backgroundStatusCount(backgroundStatus("attention")), 4);
  assert.equal(backgroundStatusCount(backgroundStatus("paused")), 0);
});

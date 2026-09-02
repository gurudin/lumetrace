import assert from "node:assert/strict";
import test from "node:test";
import {
  canActivateFileReorder,
  decideFileDragHandoff,
  hasMeaningfulFileReorderMovement,
} from "../src/pages/file-space/fileDragRouting.ts";

test("only an unfiltered automatic view can be reordered by dragging", () => {
  assert.equal(canActivateFileReorder({
    hasSearch: false,
    hasFilters: false,
    usesAutomaticOrder: true,
    visibleFileCount: 20,
  }), true);
  assert.equal(canActivateFileReorder({
    hasSearch: true,
    hasFilters: false,
    usesAutomaticOrder: true,
    visibleFileCount: 20,
  }), false);
  assert.equal(canActivateFileReorder({
    hasSearch: false,
    hasFilters: true,
    usesAutomaticOrder: true,
    visibleFileCount: 20,
  }), false);
  assert.equal(canActivateFileReorder({
    hasSearch: false,
    hasFilters: false,
    usesAutomaticOrder: false,
    visibleFileCount: 20,
  }), false);
  assert.equal(canActivateFileReorder({
    hasSearch: false,
    hasFilters: false,
    usesAutomaticOrder: true,
    visibleFileCount: 1,
  }), false);
});

test("click jitter never reaches the manual reorder commitment distance", () => {
  assert.equal(hasMeaningfulFileReorderMovement({
    startX: 100,
    startY: 100,
    currentX: 111,
    currentY: 107,
    minimumDistance: 24,
  }), false);
  assert.equal(hasMeaningfulFileReorderMovement({
    startX: 100,
    startY: 100,
    currentX: 124,
    currentY: 100,
    minimumDistance: 24,
  }), true);
});

test("a later press can still become a drag after crossing the movement threshold", () => {
  assert.equal(hasMeaningfulFileReorderMovement({
    startX: 100,
    startY: 100,
    currentX: 130,
    currentY: 100,
    minimumDistance: 24,
  }), true);
});

test("Command only hands an active single-file desktop drag to macOS", () => {
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 1,
    desktop: true,
    trigger: "command",
  }), "promote");
  assert.equal(decideFileDragHandoff({
    phase: "pending",
    fileCount: 1,
    desktop: true,
    trigger: "command",
  }), "ignore");
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 2,
    desktop: true,
    trigger: "command",
  }), "ignore");
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 1,
    desktop: false,
    trigger: "command",
  }), "ignore");
});

test("window blur promotes an eligible drag and cancels every other gesture", () => {
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 1,
    desktop: true,
    trigger: "blur",
  }), "promote");
  assert.equal(decideFileDragHandoff({
    phase: "pending",
    fileCount: 1,
    desktop: true,
    trigger: "blur",
  }), "cancel");
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 2,
    desktop: true,
    trigger: "blur",
  }), "cancel");
  assert.equal(decideFileDragHandoff({
    phase: "reordering",
    fileCount: 1,
    desktop: false,
    trigger: "blur",
  }), "cancel");
});

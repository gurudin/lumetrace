import assert from "node:assert/strict";
import test from "node:test";
import { nextFileIdForKeyboard } from "../src/pages/file-space/fileKeyboardNavigation.ts";

const placements = {
  first: { x: 0, y: 0, width: 90, previewHeight: 80 },
  second: { x: 100, y: 0, width: 180, previewHeight: 80 },
  third: { x: 290, y: 0, width: 90, previewHeight: 80 },
  fourth: { x: 0, y: 120, width: 160, previewHeight: 80 },
  fifth: { x: 170, y: 120, width: 210, previewHeight: 80 },
};
const orderedIds = Object.keys(placements);

test("moves horizontally through adaptive rows and crosses row boundaries", () => {
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "second", direction: "right", layoutMode: "adaptive" }), "third");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "third", direction: "right", layoutMode: "adaptive" }), "fourth");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "fourth", direction: "left", layoutMode: "adaptive" }), "third");
});

test("moves vertically to the nearest visual center", () => {
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "first", direction: "down", layoutMode: "adaptive" }), "fourth");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "third", direction: "down", layoutMode: "adaptive" }), "fifth");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "fifth", direction: "up", layoutMode: "adaptive" }), "third");
});

test("keeps list navigation vertical and selects the first file without an anchor", () => {
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: null, direction: "down", layoutMode: "list" }), "first");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "second", direction: "down", layoutMode: "list" }), "third");
  assert.equal(nextFileIdForKeyboard({ orderedIds, placements, currentId: "second", direction: "right", layoutMode: "list" }), null);
});

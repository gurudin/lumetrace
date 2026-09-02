import assert from "node:assert/strict";
import test from "node:test";
import {
  combineMarqueeSelection,
  normalizeSelectionRectangle,
  pointInScrollContent,
  pruneSelection,
  rectanglesIntersect,
  resolveFileClickSelection,
  selectionVisibilityWithPendingReveal,
} from "../src/pages/file-space/fileSelection.ts";

test("plain, toggle, and range clicks follow macOS selection semantics", () => {
  const orderedIds = ["a", "b", "c", "d"];
  const plain = resolveFileClickSelection({
    currentIds: new Set(["a", "b"]),
    orderedIds,
    targetId: "c",
    anchorId: "a",
    toggle: false,
    range: false,
  });
  assert.deepEqual([...plain.ids], ["c"]);

  const toggled = resolveFileClickSelection({
    currentIds: plain.ids,
    orderedIds,
    targetId: "a",
    anchorId: plain.anchorId,
    toggle: true,
    range: false,
  });
  assert.deepEqual([...toggled.ids], ["c", "a"]);

  const ranged = resolveFileClickSelection({
    currentIds: toggled.ids,
    orderedIds,
    targetId: "d",
    anchorId: "a",
    toggle: false,
    range: true,
  });
  assert.deepEqual([...ranged.ids], orderedIds);
});

test("additive range preserves the existing selection", () => {
  const result = resolveFileClickSelection({
    currentIds: new Set(["x"]),
    orderedIds: ["a", "b", "c", "d"],
    targetId: "d",
    anchorId: "b",
    toggle: true,
    range: true,
  });
  assert.deepEqual([...result.ids], ["b", "c", "d", "x"]);
});

test("marquee rectangles normalize reverse drags and require an actual overlap", () => {
  const marquee = normalizeSelectionRectangle(90, 80, 10, 20);
  assert.deepEqual(marquee, { left: 10, top: 20, right: 90, bottom: 80 });
  assert.equal(rectanglesIntersect(marquee, { left: 30, top: 30, right: 40, bottom: 40 }), true);
  assert.equal(rectanglesIntersect(marquee, { left: 90, top: 30, right: 110, bottom: 40 }), false);
});

test("marquee points use the whole scrolling content area, including outer blank space", () => {
  const bounds = {
    left: 230,
    top: 52,
    scrollLeft: 0,
    scrollTop: 180,
    scrollWidth: 760,
    scrollHeight: 1600,
  };
  assert.deepEqual(pointInScrollContent(238, 70, bounds), { x: 8, y: 198 });
  assert.deepEqual(pointInScrollContent(120, 20, bounds), { x: 0, y: 148 });
  assert.deepEqual(pointInScrollContent(1200, 1900, bounds), { x: 760, y: 1600 });
});

test("marquee replace, union, and toggle always derive from the baseline", () => {
  const baseline = new Set(["a", "b"]);
  const hits = new Set(["b", "c"]);
  assert.deepEqual([...combineMarqueeSelection(baseline, hits, "replace")], ["b", "c"]);
  assert.deepEqual([...combineMarqueeSelection(baseline, hits, "union")], ["a", "b", "c"]);
  assert.deepEqual([...combineMarqueeSelection(baseline, hits, "toggle")], ["a", "c"]);
});

test("hidden files are pruned from selection", () => {
  assert.deepEqual(
    [...pruneSelection(new Set(["a", "b", "c"]), new Set(["b", "c", "d"]))],
    ["b", "c"],
  );
});

test("a file remains selectable while navigation switches to its folder", () => {
  const visibleIds = selectionVisibilityWithPendingReveal(["old-folder-file"], "target-file");
  assert.deepEqual([...pruneSelection(new Set(["target-file"]), visibleIds)], ["target-file"]);
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  calculateJustifiedFileLayout,
  justifiedFileLayoutsEqual,
  reorderFileIdsForDraggedCard,
} from "../src/pages/file-space/fileJustifiedLayout.ts";

const baseOptions = {
  targetPreviewHeight: 100,
  horizontalGap: 10,
  verticalGap: 20,
  previewDetailsGap: 8,
  detailsHeight: 42,
};

test("fills every completed row without changing the preview height", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 500,
    items: Array.from({ length: 5 }, (_, index) => ({
      id: `file-${index}`,
      aspectRatio: 1.5,
    })),
  });

  assert.equal(layout.rowCount, 2);
  assert.deepEqual(layout.placements["file-0"], { x: 0, y: 0, width: 160, previewHeight: 100 });
  assert.deepEqual(layout.placements["file-1"], { x: 170, y: 0, width: 160, previewHeight: 100 });
  assert.deepEqual(layout.placements["file-2"], { x: 340, y: 0, width: 160, previewHeight: 100 });
  assert.equal(layout.placements["file-2"].x + layout.placements["file-2"].width, 500);
});

test("keeps the final row left aligned at natural preview widths", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 500,
    items: Array.from({ length: 5 }, (_, index) => ({
      id: `file-${index}`,
      aspectRatio: 1.5,
    })),
  });

  assert.deepEqual(layout.placements["file-3"], { x: 0, y: 170, width: 150, previewHeight: 100 });
  assert.deepEqual(layout.placements["file-4"], { x: 160, y: 170, width: 150, previewHeight: 100 });
  assert.equal(layout.height, 320);
});

test("uses aspect ratios to determine how many files fit in a row", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 500,
    items: [
      { id: "portrait", aspectRatio: 0.6 },
      { id: "square", aspectRatio: 1 },
      { id: "landscape", aspectRatio: 1.8 },
      { id: "next-row", aspectRatio: 2.2 },
    ],
  });

  assert.equal(layout.rowCount, 2);
  assert.equal(layout.placements.portrait.previewHeight, layout.placements.landscape.previewHeight);
  assert.equal(layout.placements["next-row"].y, 170);
});

test("clamps extreme aspect ratios and a card wider than its container", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 90,
    items: [
      { id: "panorama", aspectRatio: 8 },
      { id: "portrait", aspectRatio: 0.1 },
    ],
  });

  assert.equal(layout.placements.panorama.width, 90);
  assert.ok(Math.abs(layout.placements.portrait.width - 58) < Number.EPSILON * 64);
  assert.equal(layout.placements.portrait.y, 170);
});

test("reserves one shared details height so every row remains aligned", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 300,
    detailsHeight: 64,
    items: [
      { id: "first", aspectRatio: 2 },
      { id: "second", aspectRatio: 2 },
    ],
  });

  assert.equal(layout.cardHeight, 172);
  assert.equal(layout.placements.second.y, 192);
  assert.equal(layout.height, 364);
});

test("detects when a calculated layout has not changed", () => {
  const layout = calculateJustifiedFileLayout({
    ...baseOptions,
    containerWidth: 500,
    items: [{ id: "file", aspectRatio: 1 }],
  });

  assert.equal(justifiedFileLayoutsEqual(layout, { ...layout }), true);
  assert.equal(justifiedFileLayoutsEqual(layout, { ...layout, height: layout.height + 1 }), false);
});

const reorderCandidates = [
  { id: "first", left: 0, right: 100, top: 0, bottom: 160 },
  { id: "second", left: 110, right: 210, top: 0, bottom: 160 },
  { id: "third", left: 220, right: 320, top: 0, bottom: 160 },
  { id: "fourth", left: 330, right: 430, top: 0, bottom: 160 },
  { id: "fifth", left: 0, right: 100, top: 180, bottom: 340 },
];

test("places a dragged card at the start from its visual center, independent of grab offset", () => {
  const orderedIds = ["first", "second", "third", "fourth", "fifth", "dragged"];
  const reordered = reorderFileIdsForDraggedCard({
    orderedIds,
    draggedId: "dragged",
    candidates: reorderCandidates,
    draggedCenterX: 42,
    draggedCenterY: 80,
  });

  assert.deepEqual(reordered, ["dragged", "first", "second", "third", "fourth", "fifth"]);
});

test("keeps row boundaries deterministic when moving between rows", () => {
  const orderedIds = ["first", "second", "third", "fourth", "fifth", "dragged"];
  const reordered = reorderFileIdsForDraggedCard({
    orderedIds,
    draggedId: "dragged",
    candidates: reorderCandidates,
    draggedCenterX: 180,
    draggedCenterY: 260,
  });

  assert.deepEqual(reordered, ["first", "second", "third", "fourth", "fifth", "dragged"]);
});

import assert from "node:assert/strict";
import test from "node:test";
import { calculateWheelZoom, normalizeWheelDelta } from "../src/pages/file-space/imagePreviewZoom.ts";

test("keeps small trackpad movements gradual", () => {
  const zoomedIn = calculateWheelZoom(1, -4);
  const zoomedOut = calculateWheelZoom(1, 4);

  assert.ok(zoomedIn > 1 && zoomedIn < 1.01);
  assert.ok(zoomedOut < 1 && zoomedOut > 0.99);
});

test("normalizes line and page wheel deltas", () => {
  assert.equal(calculateWheelZoom(1, -1, 1), calculateWheelZoom(1, -16, 0));
  assert.equal(calculateWheelZoom(1, -1, 2), calculateWheelZoom(1, -120, 0));
});

test("limits unusually large wheel events", () => {
  assert.equal(calculateWheelZoom(1, -10_000), calculateWheelZoom(1, -100));
  assert.equal(calculateWheelZoom(1, 10_000), calculateWheelZoom(1, 100));
});

test("clamps wheel zoom to the supported range", () => {
  assert.equal(calculateWheelZoom(4, -120), 4);
  assert.equal(calculateWheelZoom(0.25, 120), 0.25);
});

test("keeps a ten-pixel gesture adjustment near half a percent", () => {
  const nextZoom = calculateWheelZoom(1, -10);
  assert.ok(nextZoom > 1.004 && nextZoom < 1.006);
});

test("caps one frame of fast input near five percent", () => {
  const nextZoom = calculateWheelZoom(1, -100);
  assert.ok(nextZoom > 1.04 && nextZoom < 1.06);
});

test("provides normalized deltas that can be accumulated once per frame", () => {
  const accumulated = normalizeWheelDelta(-4, 0) + normalizeWheelDelta(-6, 0);
  assert.equal(calculateWheelZoom(1, accumulated), calculateWheelZoom(1, -10));
});

test("preserves total gesture distance when events are split", () => {
  const oneEvent = calculateWheelZoom(1, -10);
  let splitEvents = 1;
  for (let index = 0; index < 10; index += 1) {
    splitEvents = calculateWheelZoom(splitEvents, -1);
  }

  assert.ok(Math.abs(oneEvent - splitEvents) < 1e-12);
});

test("ignores zero and non-finite wheel deltas", () => {
  assert.equal(calculateWheelZoom(1.5, 0), 1.5);
  assert.equal(calculateWheelZoom(1.5, Number.NaN), 1.5);
  assert.equal(calculateWheelZoom(1.5, Number.POSITIVE_INFINITY), 1.5);
});

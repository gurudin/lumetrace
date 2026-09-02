import assert from "node:assert/strict";
import test from "node:test";
import { shouldShowVersionTimelineByDefault } from "../src/pages/file-space/versionTimelineVisibility.ts";

test("opens a version timeline by default only for multiple versions", () => {
  assert.equal(shouldShowVersionTimelineByDefault(0), false);
  assert.equal(shouldShowVersionTimelineByDefault(1), false);
  assert.equal(shouldShowVersionTimelineByDefault(2), true);
  assert.equal(shouldShowVersionTimelineByDefault(8), true);
});

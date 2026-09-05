import assert from "node:assert/strict";
import test from "node:test";
import { shouldShowVersionTimelineByDefault } from "../src/pages/file-space/versionTimelineVisibility.ts";

test("reveals the initial timeline while leaving untracked previews unchanged", () => {
  assert.equal(shouldShowVersionTimelineByDefault(0), false);
  assert.equal(shouldShowVersionTimelineByDefault(1), true);
  assert.equal(shouldShowVersionTimelineByDefault(2), true);
  assert.equal(shouldShowVersionTimelineByDefault(8), true);
});

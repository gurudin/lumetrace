import assert from "node:assert/strict";
import test from "node:test";
import { fileSpaceRootView } from "../src/pages/file-space/fileSpaceRootPresentation.ts";

test("presents an initial load failure instead of the unconfigured setup screen", () => {
  assert.equal(fileSpaceRootView(false, "database unavailable", "unconfigured"), "loadError");
  assert.equal(fileSpaceRootView(false, null, "unconfigured"), "setup");
});

test("keeps loading and ready workspace states distinct", () => {
  assert.equal(fileSpaceRootView(true, "database unavailable", "unconfigured"), "loading");
  assert.equal(fileSpaceRootView(false, null, "ready"), "workspace");
});

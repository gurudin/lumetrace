import assert from "node:assert/strict";
import test from "node:test";
import {
  semanticStatusView,
  shouldPollSemanticStatus,
} from "../src/pages/file-space/semanticStatusPresentation.ts";

test("does not present a fake not-installed state before status is loaded", () => {
  assert.equal(semanticStatusView("idle"), "loading");
  assert.equal(semanticStatusView("loading"), "loading");
  assert.equal(semanticStatusView("error"), "error");
  assert.equal(semanticStatusView("ready"), "status");
});

test("polls only active semantic pipelines with a readable status", () => {
  assert.equal(shouldPollSemanticStatus("ready", "downloading"), true);
  assert.equal(shouldPollSemanticStatus("ready", "validating"), true);
  assert.equal(shouldPollSemanticStatus("ready", "indexing"), true);
  assert.equal(shouldPollSemanticStatus("ready", "indexing", true), false);
  assert.equal(shouldPollSemanticStatus("ready", "ready"), false);
  assert.equal(shouldPollSemanticStatus("error", "indexing"), false);
});

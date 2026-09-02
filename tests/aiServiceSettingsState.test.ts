import assert from "node:assert/strict";
import test from "node:test";
import { defaultAiServiceMode } from "../src/pages/file-space/aiServiceSettingsState.ts";

test("AI service settings always opens on the local model tab", () => {
  assert.equal(defaultAiServiceMode, "local");
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  aiConfigurationLoadResolved,
  aiHistoryLoadFailed,
  aiHistoryLoadSucceeded,
} from "../src/pages/file-space/aiHistoryLoadState.ts";

interface TestSettings {
  mode: "local";
  model: string;
}

test("history failure preserves a valid AI service configuration", () => {
  const settings: TestSettings = { mode: "local", model: "qwen3.5:27b" };
  const configured = aiConfigurationLoadResolved(settings, true);
  const failedHistory = aiHistoryLoadFailed(configured);

  assert.equal(failedHistory.configurationState, "configured");
  assert.equal(failedHistory.serviceSettings, settings);
  assert.equal(failedHistory.historyState, "error");
});

test("retrying history can recover without changing the AI service", () => {
  const settings: TestSettings = { mode: "local", model: "qwen3.5:27b" };
  const failedHistory = aiHistoryLoadFailed(aiConfigurationLoadResolved(settings, true));
  const loadedHistory = aiHistoryLoadSucceeded(failedHistory);

  assert.equal(loadedHistory.configurationState, "configured");
  assert.equal(loadedHistory.serviceSettings, settings);
  assert.equal(loadedHistory.historyState, "ready");
});

test("an unconfigured service does not start a history load", () => {
  const settings: TestSettings = { mode: "local", model: "" };
  const unconfigured = aiConfigurationLoadResolved(settings, false);

  assert.equal(unconfigured.configurationState, "unconfigured");
  assert.equal(unconfigured.historyState, "idle");
});

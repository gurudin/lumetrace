import assert from "node:assert/strict";
import test from "node:test";
import {
  canAskConfiguredAiService,
  defaultAiServiceMode,
  defaultLocalBaseUrl,
  isAiServiceConfigured,
} from "../src/pages/file-space/aiServiceSettingsState.ts";

test("AI service settings always opens on the local model tab", () => {
  assert.equal(defaultAiServiceMode, "local");
});

test("a new local model configuration does not assume a Base URL", () => {
  assert.equal(defaultLocalBaseUrl, "");
});

test("a saved cloud model and API key count as an active AI service", () => {
  assert.equal(isAiServiceConfigured({
    mode: "cloud",
    cloud: {
      model: "gpt-5-mini",
      hasApiKey: true,
    },
  }), true);
  assert.equal(isAiServiceConfigured({
    mode: "cloud",
    cloud: {
      model: "gpt-5-mini",
      hasApiKey: false,
    },
  }), false);
});

test("all four read-only Agent CLIs can run file-space questions", () => {
  for (const cli of ["claude", "hermes", "codex", "opencode"]) {
    assert.equal(canAskConfiguredAiService({
      mode: "agentCli",
      agentCli: { cli, permission: "readOnly" },
    }), true);
  }
  assert.equal(canAskConfiguredAiService({
    mode: "agentCli",
    agentCli: { cli: "claude", permission: "readWrite" },
  }), false);
  assert.equal(canAskConfiguredAiService({
    mode: "agentCli",
    agentCli: { cli: "unknown", permission: "readOnly" },
  }), false);
});

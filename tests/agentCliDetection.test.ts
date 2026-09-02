import assert from "node:assert/strict";
import test from "node:test";
import {
  agentCheckState,
  clearAgentCliRuntimeSuccess,
  recordAgentCliRuntimeSuccess,
  wasAgentCliRecentlySuccessful,
} from "../src/pages/file-space/agentCliDetection.ts";

test("does not present a transient CLI check failure as missing configuration", () => {
  assert.equal(agentCheckState({
    key: "hermes",
    installed: true,
    reachable: false,
    checkState: "checkFailed",
  }), "error");
});

test("keeps actual configuration and installation states distinct", () => {
  assert.equal(agentCheckState({
    key: "hermes",
    installed: true,
    reachable: false,
    checkState: "notConfigured",
  }), "installed");
  assert.equal(agentCheckState({
    key: "claude",
    installed: false,
    reachable: false,
    checkState: "missing",
  }), "missing");
  assert.equal(agentCheckState({
    key: "codex",
    installed: true,
    reachable: true,
    checkState: "passed",
  }), "passed");
});

test("supports status responses from an older backend", () => {
  assert.equal(agentCheckState({ key: "codex", installed: true, reachable: true }), "passed");
  assert.equal(agentCheckState({ key: "hermes", installed: true, reachable: false }), "installed");
});

test("remembers a real runtime success briefly so stale UI checks cannot override it", () => {
  clearAgentCliRuntimeSuccess();
  recordAgentCliRuntimeSuccess("hermes", 10_000);
  assert.equal(wasAgentCliRecentlySuccessful("hermes", 10_001), true);
  assert.equal(wasAgentCliRecentlySuccessful("codex", 10_001), false);
  assert.equal(wasAgentCliRecentlySuccessful("hermes", 310_001), false);
  clearAgentCliRuntimeSuccess();
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  openAiServiceSettingsEventName,
  openBackgroundStatusEventName,
  preferencesSectionForOpenEvent,
  preferencesSectionKeys,
} from "../src/pages/file-space/preferencesNavigation.ts";

test("unified preferences exposes AI and background sections in the requested order", () => {
  assert.deepEqual(preferencesSectionKeys, [
    "appearance",
    "general",
    "aiService",
    "background",
  ]);
});

test("settings shortcuts route to their unified preferences sections", () => {
  assert.equal(openAiServiceSettingsEventName, "lumetrace:open-ai-service-settings");
  assert.equal(openBackgroundStatusEventName, "lumetrace:open-background-status");
  assert.equal(
    preferencesSectionForOpenEvent(openAiServiceSettingsEventName),
    "aiService",
  );
  assert.equal(
    preferencesSectionForOpenEvent(openBackgroundStatusEventName),
    "background",
  );
  assert.equal(preferencesSectionForOpenEvent("lumetrace:unknown-settings"), null);
});

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
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

test("only an explicit archive restriction removes backup and restore without changing other settings", () => {
  const source = readFileSync(new URL("../src/pages/file-space/FileSpaceSettingsMenu.tsx", import.meta.url), "utf8");
  assert.ok(source.includes('menuItems.filter(item => backupAllowed || (item !== "backup" && item !== "restore")).map'));
  assert.ok(source.includes('const backupAllowed = useWorkspaceExtension()?.source?.capabilities?.backup !== false'));
  assert.doesNotMatch(source, /externalWorkspace/);
  const items = source.slice(source.indexOf("const menuItems:"), source.indexOf("const privacyUpdatedAt"));
  assert.deepEqual([...items.matchAll(/"([a-z]+)"/g)].map(match => match[1]), ["workspace", "preferences", "semantic", "backup", "restore", "feedback", "privacy", "about"]);
});

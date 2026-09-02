import assert from "node:assert/strict";
import test from "node:test";
import {
  globalSearchShortcutLabel,
  isGlobalSearchShortcut,
} from "../src/pages/file-space/globalSearchShortcut.ts";

const keyEvent = (overrides: Partial<Parameters<typeof isGlobalSearchShortcut>[0]> = {}) => ({
  key: "k",
  metaKey: false,
  altKey: false,
  ctrlKey: false,
  shiftKey: false,
  ...overrides,
});

test("global search uses Command K on Apple platforms", () => {
  assert.equal(globalSearchShortcutLabel("MacIntel"), "⌘K");
  assert.equal(isGlobalSearchShortcut(keyEvent({ metaKey: true }), "MacIntel"), true);
  assert.equal(isGlobalSearchShortcut(keyEvent({ altKey: true }), "MacIntel"), false);
});

test("global search uses Alt K on Windows and Linux", () => {
  assert.equal(globalSearchShortcutLabel("Win32"), "Alt K");
  assert.equal(globalSearchShortcutLabel("Linux x86_64"), "Alt K");
  assert.equal(isGlobalSearchShortcut(keyEvent({ altKey: true }), "Win32"), true);
  assert.equal(isGlobalSearchShortcut(keyEvent({ metaKey: true }), "Linux x86_64"), false);
});

test("global search ignores modified and composing shortcuts", () => {
  assert.equal(isGlobalSearchShortcut(keyEvent({ metaKey: true, shiftKey: true }), "MacIntel"), false);
  assert.equal(isGlobalSearchShortcut(keyEvent({ altKey: true, ctrlKey: true }), "Win32"), false);
  assert.equal(isGlobalSearchShortcut(keyEvent({ metaKey: true, isComposing: true }), "MacIntel"), false);
  assert.equal(isGlobalSearchShortcut(keyEvent({ key: "j", metaKey: true }), "MacIntel"), false);
});

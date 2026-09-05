import assert from "node:assert/strict";
import test from "node:test";
import { shouldPreserveNativeContextMenu } from "../src/shared/ui/nativeContextMenu.ts";

test("preserves the macOS native menu for editable and selectable text surfaces", () => {
  assert.equal(shouldPreserveNativeContextMenu("input"), true);
  assert.equal(shouldPreserveNativeContextMenu("textarea"), true);
  assert.equal(shouldPreserveNativeContextMenu("contenteditable"), true);
  assert.equal(shouldPreserveNativeContextMenu("selectable-text"), true);
});

test("keeps the native menu suppressed where Lume Trace owns the context menu", () => {
  assert.equal(shouldPreserveNativeContextMenu("other"), false);
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  helpButtonVisibilityStorageKey,
  storedHelpButtonVisibility,
} from "../src/shared/help/helpButtonVisibility.ts";

function storageWith(value: string | null) {
  return {
    getItem(key: string) {
      assert.equal(key, helpButtonVisibilityStorageKey);
      return value;
    },
  };
}

test("shows the Help button by default", () => {
  assert.equal(storedHelpButtonVisibility(storageWith(null)), true);
});

test("hides the Help button only after an explicit opt-out", () => {
  assert.equal(storedHelpButtonVisibility(storageWith("false")), false);
  assert.equal(storedHelpButtonVisibility(storageWith("true")), true);
  assert.equal(storedHelpButtonVisibility(storageWith("invalid")), true);
});

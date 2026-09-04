import assert from "node:assert/strict";
import test from "node:test";
import {
  inspectorVisibilityStorageKey,
  storedInspectorVisibility,
} from "../src/pages/file-space/fileSpaceViewPreferences.ts";

function storageWith(value: string | null) {
  return {
    getItem(key: string) {
      assert.equal(key, inspectorVisibilityStorageKey);
      return value;
    },
  };
}

test("keeps the inspector closed when no preference has been saved", () => {
  assert.equal(storedInspectorVisibility(storageWith(null)), false);
});

test("restores only an explicitly enabled inspector preference", () => {
  assert.equal(storedInspectorVisibility(storageWith("true")), true);
  assert.equal(storedInspectorVisibility(storageWith("false")), false);
  assert.equal(storedInspectorVisibility(storageWith("invalid")), false);
});

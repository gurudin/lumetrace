import test from "node:test";
import assert from "node:assert/strict";
import { splitSearchText } from "../src/pages/file-space/searchTextHighlight.ts";

test("search excerpts highlight every case-insensitive text match without HTML", () => {
  const parts = splitSearchText("Alpha 计划 and alpha notes", "alpha 计划");
  assert.deepEqual(parts.filter((part) => part.highlighted).map((part) => part.text), [
    "Alpha",
    "计划",
    "alpha",
  ]);
  assert.equal(parts.map((part) => part.text).join(""), "Alpha 计划 and alpha notes");
});

test("search excerpts preserve untrusted markup as plain text", () => {
  const value = '<img src=x onerror="bad"> Note';
  const parts = splitSearchText(value, "note");
  assert.equal(parts.map((part) => part.text).join(""), value);
  assert.equal(parts.at(-1)?.highlighted, true);
});

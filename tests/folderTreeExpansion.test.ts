import assert from "node:assert/strict";
import test from "node:test";

import {
  expandableFolderIdsInSubtree,
  isFolderSubtreeFullyExpanded,
  setFolderSubtreeExpanded,
  toggleFolderOnDoubleClick,
} from "../src/pages/file-space/folderTreeExpansion.ts";

test("double-clicking a folder with children expands exactly that folder", () => {
  const current = new Set(["already-open"]);
  const next = toggleFolderOnDoubleClick(current, "parent", true);

  assert.deepEqual([...next], ["already-open", "parent"]);
  assert.deepEqual([...current], ["already-open"]);
});

test("double-clicking an expanded folder collapses it", () => {
  const current = new Set(["parent"]);
  const next = toggleFolderOnDoubleClick(current, "parent", true);

  assert.deepEqual([...next], []);
  assert.deepEqual([...current], ["parent"]);
});

test("double-clicking a leaf does not change expansion", () => {
  const current = new Set(["parent"]);

  assert.equal(toggleFolderOnDoubleClick(current, "leaf", false), current);
});

const folders = [
  { id: "root", parentId: null },
  { id: "child", parentId: "root" },
  { id: "grandchild", parentId: "child" },
  { id: "sibling-leaf", parentId: "root" },
  { id: "outside", parentId: null },
  { id: "outside-child", parentId: "outside" },
];

test("expanding a folder subtree includes every folder that has children", () => {
  const expandableIds = expandableFolderIdsInSubtree(folders, "root");
  const current = new Set(["outside"]);
  const next = setFolderSubtreeExpanded(current, expandableIds, true);

  assert.deepEqual(new Set(expandableIds), new Set(["root", "child"]));
  assert.deepEqual(next, new Set(["outside", "root", "child"]));
  assert.deepEqual(current, new Set(["outside"]));
  assert.equal(isFolderSubtreeFullyExpanded(next, expandableIds), true);
});

test("collapsing a folder subtree preserves expansion outside that subtree", () => {
  const expandableIds = expandableFolderIdsInSubtree(folders, "root");
  const current = new Set(["root", "child", "outside"]);
  const next = setFolderSubtreeExpanded(current, expandableIds, false);

  assert.deepEqual(next, new Set(["outside"]));
  assert.deepEqual(current, new Set(["root", "child", "outside"]));
  assert.equal(isFolderSubtreeFullyExpanded(next, expandableIds), false);
});

test("leaf folders do not expose an expand-all action", () => {
  const expandableIds = expandableFolderIdsInSubtree(folders, "grandchild");

  assert.deepEqual(expandableIds, []);
  assert.equal(isFolderSubtreeFullyExpanded(new Set(), expandableIds), false);
});
